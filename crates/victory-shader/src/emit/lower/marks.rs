//! Which lanes of a wave set a write or read mark (module notes of `emit`,
//! "Write marks", "Read marks").

use super::super::bitcode::{Attr, Bin, V, pred};
use super::super::shader::flags;
use super::Lower;

// Test-only: lower the shipped per-lane marks, with no leader election,
// so a GPU test can compare both paths bit for bit. Thread-local because
// lowering runs on the test's thread and tests run concurrently.
#[cfg(test)]
thread_local! {
    pub(crate) static MARK_LEAD_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Lower `f` with per-lane marks and restore the setting afterwards.
#[cfg(test)]
pub(crate) fn per_lane_marks<T>(f: impl FnOnce() -> T) -> T {
    MARK_LEAD_OFF.with(|off| {
        let old = off.replace(true);
        let result = f();
        off.set(old);
        result
    })
}

impl Lower<'_> {
    /// Whether this lane sets mark bit `n` (its load and atomic go to the
    /// bitmap in system memory): lanes with the first active lane's bit
    /// leave it to that lane. Most lanes of a wave mark the same page, so
    /// one load crosses the bus instead of 32. In pixel shaders the lane
    /// that leads is the first that is no helper: a helper's writes are
    /// dropped.
    pub(super) fn mark_lead(&mut self, n: V) -> Option<V> {
        #[cfg(test)]
        if MARK_LEAD_OFF.with(|off| off.get()) {
            return None;
        }
        // GCN 5 (Wave64-only, `host_min_wave() >= 64`) `amdxc64.dll` hits an
        // internal `BitVector::set(0, -1)` heap-corruption bug when compiling
        // complex pixel shaders with repeated `isHelperLane` + `waveActiveOp`
        // + `waveReadLaneAt` mark-lead regions; fall back to per-lane marks in
        // pixel shaders while keeping wave coalescing on CS / emulated-MS / VS.
        if self.s.pixel && super::host_min_wave() >= 64 {
            return None;
        }
        self.s.flags |= flags::WAVE_OPS;
        let c = self.cur;
        let (i1, i32t) = (self.s.t.i1, self.s.t.i32);
        let (f, first) = if self.s.pixel {
            let helper = self
                .s
                .op(c, "isHelperLane.i1", Attr::ReadOnly, i1, 221, &[]);
            let lane = self
                .s
                .op(c, "waveGetLaneIndex", Attr::ReadNone, i32t, 111, &[]);
            // A helper's key is its lane with a high bit: the minimum is the
            // first non-helper, or (all helpers) a helper no lane matches,
            // still a lane to read at (writes there are dropped anyway).
            let flag = self.s.m.i32(1 << 30);
            let flagged = self.s.m.bin(c, Bin::Or, lane, flag);
            let key = self.s.m.select(c, helper, flagged, lane);
            // Operation 2, unsigned: the minimum.
            let (min, unsigned) = (self.s.m.i8(2), self.s.m.i8(1));
            let lead = self.s.op(
                c,
                "waveActiveOp.i32",
                Attr::NoUnwind,
                i32t,
                119,
                &[key, min, unsigned],
            );
            let low = self.s.m.i32((1 << 30) - 1);
            let at = self.s.m.bin(c, Bin::And, lead, low);
            let f = self
                .s
                .op(c, "waveReadLaneAt.i32", Attr::NoUnwind, i32t, 117, &[n, at]);
            (f, self.s.m.cmp(c, pred::EQ, lane, lead))
        } else {
            let f = self
                .s
                .op(c, "waveReadLaneFirst.i32", Attr::NoUnwind, i32t, 118, &[n]);
            (
                f,
                self.s
                    .op(c, "waveIsFirstLane", Attr::NoUnwind, i1, 110, &[]),
            )
        };
        let differ = self.s.m.cmp(c, pred::NE, n, f);
        Some(self.s.m.bin(c, Bin::Or, differ, first))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::validator::Validator;
    use super::super::{Parts, TEST_MIN_WAVE, lower};
    use crate::native::{HwStage, NativeProgram, RootParam, RootType, StageIo};
    use crate::{io, ir, resource, structure};

    struct MinWaveGuard(Option<u32>);

    impl Drop for MinWaveGuard {
        fn drop(&mut self) {
            TEST_MIN_WAVE.set(self.0);
        }
    }

    #[test]
    fn pixel_mark_lead_disabled_on_wave64_only_hosts() {
        // Minimal pixel shader that reads a heap-direct 2D texture (triggering
        // `mark_view_read` -> `mark_lead`) and exports null.
        let words = [
            0xBE84_0300u32, // s_mov_b32 s4, s0
            0xBE85_0380, // s_mov_b32 s5, 0
            0xF40C_0302,
            0xFA00_0000, // s_load_dwordx8 s[12:19], s[4:5], 0x0
            0xBE88_03FF,
            0x0000_0092, // s_mov_b32 s8, 0x92
            0xBE89_0380, // s_mov_b32 s9, 0
            0xBE8A_0380, // s_mov_b32 s10, 0
            0xBE8B_0380, // s_mov_b32 s11, 0
            0x7E04_02F0, // v_mov_b32 v2, 0.5
            0x7E06_02F0, // v_mov_b32 v3, 0.5
            0xBF8C_C07F, // s_waitcnt lgkmcnt(0)
            0xF09C_0F08,
            0x0043_0C02, // image_sample_lz v[12:15], v[2:3], s[12:19], s[8:11] dmask:0xf dim:2D
            0xBF8C_3F70, // s_waitcnt vmcnt(0)
            0xF800_1890,
            0x0000_0000, // exp null off, off, off, off done vm
            0xBF81_0000, // s_endpgm
        ];
        let native = NativeProgram {
            stage: HwStage::Ps,
            wave64: false,
            vgprs: 16,
            layout: vec![RootParam {
                root: RootType::ResourceHeap,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 0,
                slot: 0,
            }],
            used: Vec::new(),
            io: StageIo::Ps {
                inputs: Vec::new(),
                input_addr: 0,
                db_shader_control: 0,
                patches: Vec::new(),
            },
            code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        };
        let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir");
        let st = structure::structure(&p).expect("structure");
        let res = resource::track(&p, &native.layout, native.stage, Some(&st)).expect("resource");
        let io = io::raise(&p, &native).expect("io");
        let parts = Parts {
            native: &native,
            p: &p,
            st: &st,
            io: &io,
            res: &res,
            inputs: Vec::new(),
            outputs: Vec::new(),
            prims: Vec::new(),
            mesh_stride: 0,
            view_marks: true,
            emulate_mesh: false,
        };

        let _guard = MinWaveGuard(TEST_MIN_WAVE.get());
        for (min_wave, expect_wave_ops) in [(64, false), (32, true)] {
            TEST_MIN_WAVE.set(Some(min_wave));
            let lowered = lower(&parts).expect("lower");
            assert_eq!(lowered.wave_ops, expect_wave_ops, "min_wave={min_wave}");
            assert!(lowered.view_marks);
            if let Ok(mut v) = Validator::load(Validator::default_path().as_deref()) {
                v.validate(&mut lowered.dxil.clone())
                    .unwrap_or_else(|e| panic!("min_wave={min_wave}: DXIL validation: {e}"));
            }
        }
    }
}
