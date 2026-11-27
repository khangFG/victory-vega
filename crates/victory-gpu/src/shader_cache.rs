//! Translated shaders kept on disk (decision 0003): under the configured
//! cache folder, keyed by the container's content and by what the
//! translation depends on ([`build`]), so a changed translator never serves
//! an old translation, while rebuilding the rest of VIctory keeps them.
//! Each translator's entries are in a folder of their own, named by the
//! start of that key: older translators' folders can be deleted whole.
//! Entries go through `victory-pipeline`'s content-addressed cache, which
//! writes them atomically; an entry that does not read back is translated
//! again. A container that does not translate is kept too, with the reason,
//! so it is not tried again on every run.
//!
//! `victory translate --cache` fills the same cache ahead of a run, through
//! [`host_shader`].
//!
//! The entry is our own encoding of [`HostShader`]; this file's text is part
//! of the key, so changing the encoding invalidates every entry.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use victory_pipeline::{Cache, Converter};
use victory_shader::emit::{HostShader, MeshEmu, MeshMode, MeshOutput, StaticSampler, UserData};
use victory_shader::{HwStage, RootType};

static CACHE: OnceLock<Cache> = OnceLock::new();
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// Keeps translated shaders in a folder of `dir` from now on: the folder
/// of this translator, `None` if it cannot be opened.
pub fn use_shader_cache(dir: PathBuf) -> Option<PathBuf> {
    let dir = dir.join(&blake3::Hash::from(*build()).to_hex()[..16]);
    match Cache::open(&dir) {
        Ok(cache) => {
            tracing::info!(dir = %dir.display(), "shader cache");
            let _ = CACHE.set(cache);
            Some(dir)
        }
        Err(e) => {
            tracing::warn!(error = %e, "no shader cache");
            None
        }
    }
}

/// Shaders found in the cache and translated since the start.
pub fn counts() -> (u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}

/// What a translation depends on besides its container: the translator
/// (`victory_shader::SOURCE_HASH`: its sources, manifest, the lock file and
/// the compiler), the entry's encoding (this file's text) and the
/// translation settings (the loop limit, which a diagnostics build can
/// change at run time).
fn build() -> &'static [u8; 32] {
    static BUILD: OnceLock<[u8; 32]> = OnceLock::new();
    BUILD.get_or_init(|| {
        let limit = victory_shader::emit::loop_limit();
        if limit != victory_shader::emit::DEFAULT_LOOP_LIMIT {
            tracing::warn!(
                limit,
                "VICTORY_SHADER_LOOP_LIMIT: translated loops stop sooner"
            );
        }
        let opt = victory_shader::emit::shader_opt();
        if opt != victory_shader::emit::DEFAULT_SHADER_OPT {
            tracing::info!(opt, "VICTORY_SHADER_OPT: custom DXIL clean-up passes");
        }
        let constants_raw = victory_shader::emit::constants_raw();
        if constants_raw {
            tracing::info!("VICTORY_PORT_CBV=1: root constant buffers lowered to raw root SRVs");
        }
        key(
            victory_shader::SOURCE_HASH,
            ENCODING,
            limit,
            opt,
            constants_raw,
        )
    })
}

/// This file, whose `encode` and `decode` define the entries.
const ENCODING: &[u8] = include_bytes!("shader_cache.rs");

fn key(
    translator: &str,
    encoding: &[u8],
    limit: u32,
    opt: u32,
    constants_raw: bool,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(translator.as_bytes());
    h.update(blake3::hash(encoding).as_bytes());
    h.update(&limit.to_le_bytes());
    h.update(&opt.to_le_bytes());
    if constants_raw {
        h.update(b"VICTORY_PORT_CBV=1");
    }
    *h.finalize().as_bytes()
}

/// The translation of a console shader container, from the cache when
/// there is one, and whether it came from there. Mesh programs translate
/// in `mesh` mode, which has entries of its own.
pub fn host_shader(container: &[u8], mesh: MeshMode) -> (Result<HostShader, String>, bool) {
    // Writes through views mark their pages: copy-backs compare only those
    // (`crate::shadow::copy_back_marks`).
    let options = victory_shader::emit::Options {
        view_marks: true,
        mesh,
    };
    let translate =
        |c: &[u8]| victory_shader::emit::translate_with(c, options).map_err(|e| e.to_string());
    let id = match mesh {
        MeshMode::Native => "shader.dxil",
        MeshMode::Emulate => "shader.dxil.mesh-emu",
    };
    match CACHE.get() {
        Some(cache) => {
            let mut h = blake3::Hasher::new();
            h.update(build());
            h.update(&victory_shader::emit::host_caps_key());
            let keyed = *h.finalize().as_bytes();
            cached(cache, &keyed, id, container, translate)
        }
        None => (translate(container), false),
    }
}

/// The cache entry's converter: the build and the container in, the
/// encoded translation (or refusal) out. `ran` tells a miss from a hit;
/// `made` keeps a miss's translation, so the entry just written is not read
/// back (the first read of a new file costs about as much as a small
/// translation where a virus scanner looks at it: 0.5-3 ms here).
struct Translate<F> {
    id: &'static str,
    translate: F,
    ran: AtomicBool,
    made: Mutex<Option<Result<HostShader, String>>>,
}

impl<F: Fn(&[u8]) -> Result<HostShader, String> + Send + Sync> Converter for Translate<F> {
    fn id(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> u32 {
        // The build fingerprint in the input does the versioning.
        1
    }
    fn convert(&self, input: &[u8]) -> anyhow::Result<Vec<u8>> {
        self.ran.store(true, Ordering::Relaxed);
        let made = (self.translate)(&input[32..]);
        let bytes = encode(&made);
        *self.made.lock().unwrap_or_else(|e| e.into_inner()) = Some(made);
        Ok(bytes)
    }
}

fn cached(
    cache: &Cache,
    build: &[u8; 32],
    id: &'static str,
    container: &[u8],
    translate: impl Fn(&[u8]) -> Result<HostShader, String> + Send + Sync,
) -> (Result<HostShader, String>, bool) {
    let input = [&build[..], container].concat();
    let converter = Translate {
        id,
        translate,
        ran: AtomicBool::new(false),
        made: Mutex::new(None),
    };
    let made = || {
        converter
            .made
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    };
    for _ in 0..2 {
        let path = match cache.get_or_convert(&converter, &input) {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(error = format!("{e:#}"), "shader cache entry not written");
                break;
            }
        };
        if let Some(shader) = made() {
            MISSES.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(path = %path.display(), "shader translated and cached");
            return (shader, false);
        }
        if let Some(shader) = std::fs::read(&path).ok().and_then(|b| decode(&b)) {
            let hit = !converter.ran.load(Ordering::Relaxed);
            let (counter, what) = if hit {
                (&HITS, "from the cache")
            } else {
                (&MISSES, "translated and cached")
            };
            counter.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(path = %path.display(), "shader {what}");
            return (shader, hit);
        }
        tracing::debug!(path = %path.display(), "unreadable shader cache entry; translating again");
        let _ = std::fs::remove_file(&path);
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    // A translation whose entry could not be written is not made again.
    (
        made().unwrap_or_else(|| (converter.translate)(container)),
        false,
    )
}

const MAGIC: &[u8; 4] = b"VSH2";
/// A container that does not translate: the reason follows.
const REFUSED: &[u8; 4] = b"VSHE";
const STAGES: [HwStage; 8] = [
    HwStage::Ls,
    HwStage::Hs,
    HwStage::Es,
    HwStage::Gs,
    HwStage::Vs,
    HwStage::Ps,
    HwStage::Ms,
    HwStage::Cs,
];

fn root_code(r: RootType) -> u32 {
    match r {
        RootType::Constants => 0,
        RootType::Descriptor => 1,
        RootType::Table => 2,
        RootType::ImmediateSampler => 3,
        RootType::ResourceHeap => 4,
        RootType::SamplerHeap => 5,
        RootType::Other(x) => 6 + x,
    }
}

fn root_type(code: u32) -> RootType {
    match code {
        0 => RootType::Constants,
        1 => RootType::Descriptor,
        2 => RootType::Table,
        3 => RootType::ImmediateSampler,
        4 => RootType::ResourceHeap,
        5 => RootType::SamplerHeap,
        x => RootType::Other(x - 6),
    }
}

/// Little-endian words and length-prefixed lists; a refusal is its
/// reason's text.
fn encode(translation: &Result<HostShader, String>) -> Vec<u8> {
    let s = match translation {
        Ok(s) => s,
        Err(reason) => return [REFUSED.as_slice(), reason.as_bytes()].concat(),
    };
    // Every field by name: a field added to `HostShader` must be added here
    // (and to `decode`).
    let HostShader {
        stage,
        dxil,
        user_data,
        samplers,
        static_samplers,
        root_cbvs,
        root_srvs,
        root_uavs,
        memory,
        memory_writes,
        view_addresses,
        heap,
        heap_writes,
        mesh_emu,
        view_marks,
    } = s;
    let mut out = MAGIC.to_vec();
    let word = |out: &mut Vec<u8>, v: u32| out.extend(v.to_le_bytes());
    let stage = STAGES.iter().position(|x| x == stage).unwrap_or(0) as u32;
    word(&mut out, stage);
    word(&mut out, dxil.len() as u32);
    out.extend(dxil);
    word(&mut out, user_data.len() as u32);
    for u in user_data {
        for v in [u.index, u32::from(u.slot), root_code(u.root), u.dword] {
            word(&mut out, v);
        }
    }
    word(&mut out, samplers.len() as u32);
    for sampler in samplers {
        for &v in sampler {
            word(&mut out, v);
        }
    }
    word(&mut out, static_samplers.len() as u32);
    for t in static_samplers {
        let [u, v, w] = t.address;
        let words = [
            t.filter,
            u,
            v,
            w,
            t.mip_lod_bias.to_bits(),
            t.max_anisotropy,
            t.comparison,
            t.border,
            t.min_lod.to_bits(),
            t.max_lod.to_bits(),
        ];
        for v in words {
            word(&mut out, v);
        }
    }
    for slots in [root_cbvs, root_srvs, root_uavs] {
        word(&mut out, slots.len() as u32);
        out.extend(slots);
    }
    word(
        &mut out,
        u32::from(*memory)
            | u32::from(*memory_writes) << 1
            | u32::from(*view_addresses) << 2
            | u32::from(*heap) << 3
            | u32::from(*heap_writes) << 4
            | u32::from(*view_marks) << 5,
    );
    let encode_outputs = |out: &mut Vec<u8>, items: &[MeshOutput]| {
        word(out, items.len() as u32);
        for o in items {
            let name = o.name.as_bytes();
            word(out, name.len() as u32);
            out.extend(name);
            for v in [
                o.index,
                u32::from(o.kind),
                u32::from(o.comp),
                u32::from(o.interp),
                o.row.unwrap_or(u32::MAX),
                u32::from(o.col),
                u32::from(o.cols),
                o.dword_offset,
            ] {
                word(out, v);
            }
        }
    };
    match mesh_emu {
        None => word(&mut out, 0),
        Some(m) => {
            word(&mut out, 1);
            for v in [
                m.threads,
                m.max_vertices,
                m.max_primitives,
                m.vertex_dwords,
                m.prim_dwords,
                u32::from(m.reads_group_id),
            ] {
                word(&mut out, v);
            }
            encode_outputs(&mut out, &m.vertex_outputs);
            encode_outputs(&mut out, &m.prim_outputs);
        }
    }
    out
}

fn decode(bytes: &[u8]) -> Option<Result<HostShader, String>> {
    struct Reader<'a>(&'a [u8]);
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> Option<&'a [u8]> {
            let (head, rest) = self.0.split_at_checked(n)?;
            self.0 = rest;
            Some(head)
        }
        fn word(&mut self) -> Option<u32> {
            Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
        }
        fn list(&mut self) -> Option<Vec<u8>> {
            let n = self.word()? as usize;
            Some(self.take(n)?.to_vec())
        }
        fn outputs(&mut self) -> Option<Vec<MeshOutput>> {
            let n = self.word()? as usize;
            let mut out = Vec::with_capacity(n.min(256));
            for _ in 0..n {
                let name = String::from_utf8(self.list()?).ok()?;
                let index = self.word()?;
                let kind = u8::try_from(self.word()?).ok()?;
                let comp = u8::try_from(self.word()?).ok()?;
                let interp = u8::try_from(self.word()?).ok()?;
                let row = match self.word()? {
                    u32::MAX => None,
                    r => Some(r),
                };
                let col = u8::try_from(self.word()?).ok()?;
                let cols = u8::try_from(self.word()?).ok()?;
                let dword_offset = self.word()?;
                out.push(MeshOutput {
                    name,
                    index,
                    kind,
                    comp,
                    interp,
                    row,
                    col,
                    cols,
                    dword_offset,
                });
            }
            Some(out)
        }
    }
    if let Some(reason) = bytes.strip_prefix(REFUSED) {
        return Some(Err(String::from_utf8(reason.to_vec()).ok()?));
    }
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC {
        return None;
    }
    let stage = *STAGES.get(r.word()? as usize)?;
    let dxil = r.list()?;
    let n = r.word()? as usize;
    let mut user_data = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        user_data.push(UserData {
            index: r.word()?,
            slot: u8::try_from(r.word()?).ok()?,
            root: root_type(r.word()?),
            dword: r.word()?,
        });
    }
    let n = r.word()? as usize;
    let mut samplers = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        samplers.push([r.word()?, r.word()?, r.word()?, r.word()?]);
    }
    let n = r.word()? as usize;
    let mut static_samplers = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        static_samplers.push(StaticSampler {
            filter: r.word()?,
            address: [r.word()?, r.word()?, r.word()?],
            mip_lod_bias: f32::from_bits(r.word()?),
            max_anisotropy: r.word()?,
            comparison: r.word()?,
            border: r.word()?,
            min_lod: f32::from_bits(r.word()?),
            max_lod: f32::from_bits(r.word()?),
        });
    }
    let root_cbvs = r.list()?;
    let root_srvs = r.list()?;
    let root_uavs = r.list()?;
    let memory = r.word()?;
    let mesh_emu = match r.word()? {
        0 => None,
        1 => Some(MeshEmu {
            threads: r.word()?,
            max_vertices: r.word()?,
            max_primitives: r.word()?,
            vertex_dwords: r.word()?,
            prim_dwords: r.word()?,
            reads_group_id: match r.word()? {
                0 => false,
                1 => true,
                _ => return None,
            },
            vertex_outputs: r.outputs()?,
            prim_outputs: r.outputs()?,
        }),
        _ => return None,
    };
    r.0.is_empty().then_some(Ok(HostShader {
        stage,
        dxil,
        user_data,
        samplers,
        static_samplers,
        root_cbvs,
        root_srvs,
        root_uavs,
        memory: memory & 1 != 0,
        memory_writes: memory & 2 != 0,
        view_addresses: memory & 4 != 0,
        heap: memory & 8 != 0,
        heap_writes: memory & 16 != 0,
        mesh_emu,
        view_marks: memory & 32 != 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader() -> HostShader {
        HostShader {
            stage: HwStage::Ps,
            dxil: b"DXBC and the rest".to_vec(),
            user_data: vec![UserData {
                index: 8,
                slot: 1,
                root: RootType::Table,
                dword: 0,
            }],
            samplers: vec![[0x400, 0xc0_0000, 0x18f0_0000, 0]],
            static_samplers: vec![StaticSampler::from_words([
                0x400,
                0xc0_0000,
                0x18f0_0000,
                0,
            ])],
            root_cbvs: vec![2],
            root_srvs: vec![0, 3],
            root_uavs: vec![4],
            memory: true,
            memory_writes: true,
            view_addresses: true,
            heap: true,
            heap_writes: true,
            mesh_emu: None,
            view_marks: true,
        }
    }

    fn mesh_emu_shader() -> HostShader {
        HostShader {
            stage: HwStage::Ms,
            dxil: b"DXBC emulated mesh cs".to_vec(),
            user_data: Vec::new(),
            samplers: Vec::new(),
            static_samplers: Vec::new(),
            root_cbvs: Vec::new(),
            root_srvs: Vec::new(),
            root_uavs: Vec::new(),
            memory: false,
            memory_writes: false,
            view_addresses: false,
            heap: false,
            heap_writes: false,
            view_marks: false,
            mesh_emu: Some(MeshEmu {
                threads: 64,
                max_vertices: 32,
                max_primitives: 16,
                vertex_dwords: 8,
                prim_dwords: 4,
                reads_group_id: true,
                vertex_outputs: vec![
                    MeshOutput {
                        name: "SV_Position".into(),
                        index: 0,
                        kind: 3,
                        comp: 9,
                        interp: 4,
                        row: None,
                        col: 0,
                        cols: 4,
                        dword_offset: 0,
                    },
                    MeshOutput {
                        name: "COLOR".into(),
                        index: 0,
                        kind: 0,
                        comp: 9,
                        interp: 2,
                        row: Some(0),
                        col: 0,
                        cols: 4,
                        dword_offset: 4,
                    },
                ],
                prim_outputs: vec![MeshOutput {
                    name: "PRIMID".into(),
                    index: 0,
                    kind: 0,
                    comp: 5,
                    interp: 1,
                    row: Some(1),
                    col: 0,
                    cols: 1,
                    dword_offset: 0,
                }],
            }),
        }
    }

    #[test]
    fn translations_come_back_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let build = [7u8; 32];
        let container = b"a console container".as_slice();
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        assert_eq!(
            cached(&cache, &build, "shader.dxil", container, translate),
            (Ok(shader()), false)
        );
        assert_eq!(
            cached(&cache, &build, "shader.dxil", container, translate),
            (Ok(shader()), true)
        );
        assert_eq!(runs.load(Ordering::Relaxed), 1, "the second came from disk");

        // Another converter id (the mesh-emu one) caches separately.
        let emu_runs = AtomicU64::new(0);
        let emu_translate = |_: &[u8]| {
            emu_runs.fetch_add(1, Ordering::Relaxed);
            Ok(mesh_emu_shader())
        };
        assert_eq!(
            cached(
                &cache,
                &build,
                "shader.dxil.mesh-emu",
                container,
                emu_translate
            ),
            (Ok(mesh_emu_shader()), false)
        );
        assert_eq!(emu_runs.load(Ordering::Relaxed), 1);

        // Another build translates again.
        assert_eq!(
            cached(&cache, &[8; 32], "shader.dxil", container, translate).0,
            Ok(shader())
        );
        assert_eq!(runs.load(Ordering::Relaxed), 2);

        // A damaged entry is a miss, translated and stored again.
        for e in walk(dir.path()) {
            std::fs::write(e, b"VSH2 broken").unwrap();
        }
        assert_eq!(
            cached(&cache, &build, "shader.dxil", container, translate),
            (Ok(shader()), false)
        );
        assert_eq!(runs.load(Ordering::Relaxed), 3);
        assert_eq!(
            cached(&cache, &build, "shader.dxil", container, translate),
            (Ok(shader()), true)
        );
        assert_eq!(runs.load(Ordering::Relaxed), 3);

        // Refusals are stored with their reason, and not tried again.
        let fails = AtomicU64::new(0);
        let fail = |_: &[u8]| {
            fails.fetch_add(1, Ordering::Relaxed);
            Err::<HostShader, _>("unsupported".to_owned())
        };
        let refused = (Err("unsupported".to_owned()), false);
        assert_eq!(
            cached(&cache, &build, "shader.dxil", b"other", fail),
            refused
        );
        assert_eq!(
            cached(&cache, &build, "shader.dxil", b"other", fail),
            (refused.0, true)
        );
        assert_eq!(fails.load(Ordering::Relaxed), 1);
    }

    /// A translation whose entry cannot be written (the cache folder is
    /// a file now) is handed back as it was made, not made again.
    #[test]
    fn a_translation_is_made_once_when_its_entry_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        let cache = Cache::open(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::write(&root, b"not a folder").unwrap();
        let runs = AtomicU64::new(0);
        let translate = |_: &[u8]| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(shader())
        };
        assert_eq!(
            cached(&cache, &[7; 32], "test", b"a container", translate),
            (Ok(shader()), false)
        );
        assert_eq!(runs.load(Ordering::Relaxed), 1);
    }

    /// The key's parts: a change to any of them (the translator's sources,
    /// this file's encoding, the loop limit) makes another key, so no entry
    /// written before it is read.
    #[test]
    fn the_key_covers_translator_encoding_and_settings() {
        let opt = victory_shader::emit::DEFAULT_SHADER_OPT;
        let k = key("0123456789abcdef", b"encoding", 1 << 20, opt, false);
        assert_eq!(k, key("0123456789abcdef", b"encoding", 1 << 20, opt, false));
        assert_ne!(k, key("0123456789abcdee", b"encoding", 1 << 20, opt, false));
        assert_ne!(k, key("0123456789abcdef", b"encoding.", 1 << 20, opt, false));
        assert_ne!(k, key("0123456789abcdef", b"encoding", 1 << 19, opt, false));
        assert_ne!(
            k,
            key("0123456789abcdef", b"encoding", 1 << 20, opt ^ 1, false)
        );
        assert_ne!(k, key("0123456789abcdef", b"encoding", 1 << 20, opt, true));
        assert_eq!(
            *build(),
            key(
                victory_shader::SOURCE_HASH,
                ENCODING,
                victory_shader::emit::loop_limit(),
                victory_shader::emit::shader_opt(),
                victory_shader::emit::constants_raw(),
            )
        );
        // The encoding part is this file as built.
        assert!(
            std::str::from_utf8(ENCODING)
                .unwrap()
                .contains("fn encode(translation: &Result<HostShader, String>)")
        );
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_dir() {
                files.extend(walk(&e.path()));
            } else {
                files.push(e.path());
            }
        }
        files
    }

    #[test]
    fn entries_round_trip_and_reject_damage() {
        for s in [shader(), mesh_emu_shader()] {
            let bytes = encode(&Ok(s.clone()));
            assert_eq!(decode(&bytes), Some(Ok(s)));
            assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
            assert_eq!(decode(&[bytes.as_slice(), &[0]].concat()), None);
        }
        assert_eq!(decode(b"VSH1"), None);
        let refusal = Err("GFX10 code at 0x10: x is not supported".to_owned());
        assert_eq!(decode(&encode(&refusal)), Some(refusal));
        assert_eq!(decode(&[b"VSHE".as_slice(), &[0xff]].concat()), None);
    }

    /// A local debugging aid: dumps the bindings of the mesh (emulated)
    /// entries in a real run's cache, so the draw path's root parameters can
    /// be checked against what the title's shaders actually read. Run with
    /// `cargo test -p victory-gpu dump_cached_mesh -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a local run's shader cache"]
    fn dump_cached_mesh_entries() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let dir = std::path::Path::new(&local).join("victory").join("cache");
        let entries = walk(&dir);
        let mut mesh = 0;
        for e in &entries {
            let Ok(bytes) = std::fs::read(e) else {
                continue;
            };
            let Some(Ok(s)) = decode(&bytes) else {
                continue;
            };
            if s.stage != HwStage::Ms {
                continue;
            }
            mesh += 1;
            println!(
                "=== {} ({}) mesh_emu: {}",
                e.display(),
                s.dxil.len(),
                s.mesh_emu.is_some()
            );
            println!(
                "user_data ({}): {:?}",
                s.user_data.len(),
                s.user_data
                    .iter()
                    .map(|u| format!("dword {} slot {} {:?}+{}", u.index, u.slot, u.root, u.dword))
                    .collect::<Vec<_>>()
            );
            println!("root_cbvs {:?} root_srvs {:?}", s.root_cbvs, s.root_srvs);
            println!(
                "samplers {} static {}",
                s.samplers.len(),
                s.static_samplers.len()
            );
            if let Some(m) = &s.mesh_emu {
                println!(
                    "mesh_emu: threads {} verts {} prims {} vert_dwords {} prim_dwords {} \
                     reads_group_id {} group_bytes {}",
                    m.threads,
                    m.max_vertices,
                    m.max_primitives,
                    m.vertex_dwords,
                    m.prim_dwords,
                    m.reads_group_id,
                    m.group_bytes()
                );
                println!(
                    "vertex outputs: {:?}",
                    m.vertex_outputs
                        .iter()
                        .map(|o| format!("{}@{}", o.name, o.dword_offset))
                        .collect::<Vec<_>>()
                );
                println!(
                    "prim outputs: {:?}",
                    m.prim_outputs
                        .iter()
                        .map(|o| format!("{}@{}", o.name, o.dword_offset))
                        .collect::<Vec<_>>()
                );
            }
        }
        println!("mesh entries: {mesh} of {}", entries.len());
    }
}
