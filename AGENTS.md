# AGENTS.md — VIctory-Vega

Instructions for AI coding agents and developers working in `VIctory-Vega`.

## 1. Mission & Target Hardware Profile

`VIctory-Vega` is a research fork of VIctory combining `victory-polaris` (base runtime, mesh emulation, BCPack, offline pipeline prebuild, Series S patches) with key capabilities harvested from `VIctory-RDNA1-Hybrid` plus new **Wave32-on-Wave64 compute lowering** specifically for **AMD Radeon RX Vega (GCN 5)** GPUs — including low-VRAM Vega APUs (e.g. `0x15E7`, ~512 MiB–2 GiB dedicated VRAM) and discrete Vega 56/64.

| Capability | AMD Vega (GCN 5) | How `VIctory-Vega` handles it |
|---|---|---|
| Direct3D 12 Shader Model | SM 6.6 | Required; dynamic resources enabled |
| Hardware Mesh Shaders | None (`NotSupported`) | Compute + unpack-VS emulation (`mesh_emu.rs`, `MeshMode::Emulate`) |
| Native Wave Size (`WaveLaneCountMin..Max`) | **64..64 (Wave64-only)** | Console Wave32 compute shaders lower as two isolated 32-lane halves inside `WaveSize(64)` (`wave32_in_64` in `crates/victory-shader/src/emit/lower.rs` and `lower_vega.rs`) |
| Structured Buffer Bounds (`V#.NUM_RECORDS`) | **Element count** (not bytes as on RDNA 2+) | `8404297`: 4-byte-aligned structured buffer ops & host SRV/UAV views unify onto raw `R32_TYPELESS` + `FLAG_RAW` (`lower_vega.rs`, `lower.rs`, `descriptor/host.rs`) with 16-byte `view_copy_align` while preserving `VIEW_STRIDES` |
| Pixel-Shader Barycentrics (`SV_Barycentrics`) | **None (`BarycentricsSupported = false`)** | Harvested from `VIctory-RDNA1-Hybrid`: probes `D3D12_FEATURE_D3D12_OPTIONS3`, omits `SV_Barycentrics` / `SFI0` `0x20000`, falls back to hardware interpolation; custom per-vertex/weighted cases fail explicitly |
| 64-bit Typed Heap Atomics (`OPTIONS9/11`) | Usually unsupported | Refused cleanly by `Host::heap_atomics64()` rather than crashing D3D12 |
| Dedicated VRAM (Vega APU / iGPU) | 512 MiB–2 GiB UMA | `residency.rs` tracks `GlobalMemoryStatusEx` + `E_OUTOFMEMORY` 500 ms backoff; `mesh_emu.rs` automatically scales default output buffer down to `16 / 8 / 4 MiB` when `DedicatedVideoMemory <= 2 GiB` |

## 2. Provenance (What Came From Where)

- **Base (`victory-polaris`):**
  - `crates/victory-formats/src/bcpack-tables.bin` & `bcpack.rs` (BCPack texture decompression)
  - `crates/victory-game/src/lib.rs`, `shaderpack.rs`, `pipelines.rs` & `apps/victory/src/prebuild.rs` (Series S `UPSCALER_6` patch, offline PSO prebuild)
  - `crates/victory-gpu/src/mesh_emu.rs`, `shadow.rs`, `memory.rs`, `texback.rs`, `ambient.rs`, `draw.rs` (`VICTORY_MESH_BUFFER_MB`, loose shadow eviction, `VICTORY_TEXTURE_BACK`, `VICTORY_BIND_CACHE`, `VICTORY_AMBIENT`)
  - `crates/victory-shader` GFX10 translation fixes including 4-bit `v_permlanex16_b32`
- **Harvested from `VIctory-RDNA1-Hybrid`:**
  - `D3D12_FEATURE_D3D12_OPTIONS3::BarycentricsSupported` runtime probe and non-barycentric PS fallback (`crates/victory-gpu/src/host.rs`, `crates/victory-shader/src/emit/lower.rs`)
  - `D3D12_FEATURE_D3D12_OPTIONS1` wave-lane capability probing (`host.rs`, `emit::host`)
  - `crates/victory-gpu/src/residency.rs` (Windows physical/commit pressure tracking + `E_OUTOFMEMORY` upload backoff)
- **New for Vega (`VIctory-Vega`):**
  - **Wave32-on-Wave64 compute lowering (`wave32_in_64`):** `host_min_wave()` probe + `WaveSize(64)` emission with half-wave isolation (`lane & 31`, `(lane & 31) | half_base`, per-half `ballot_at`, `v_readfirstlane_b32` via `firstbitlow(ballot)`, 5-step butterfly `half_wave_reduce_32`, 5-step Hillis-Steele `half_wave_prefix_32`)
  - **Host-capability-aware cache keying:** `emit::host_caps_key()` hashed into both `shader_cache.rs` and `pipeline/library.rs` so Vega-lowered DXIL never collides with RDNA2/WARP caches
  - **Low-VRAM mesh emulation defaults:** automatic `16 / 8 / 4 MiB` fallback sizing when `DedicatedVideoMemory <= 2 GiB`
  - **Upstream sync (`Marvisak/VIctory` `63bcca4`):** Placed texture heaps (`placement.rs`), VRAM-budget LRU eviction (`evict_unused`), lazy texture/bindless creation, shared upload staging & per-thread `Scratch`, deferred `host_writes`, wave64 `ds_bpermute_b32` 32-lane half scoping, and shader/watch correctness fixes
  - **On-demand mip tile allocation (`tiles.rs`, upstream PR `#2389` merged natively atop `63bcca4`):** opt-in `VICTORY_LEVELS_ON_DEMAND=1` uses D3D12 `CreateReservedResource` / `UpdateTileMappings` on Tiled Resources Tier 2+ GPUs so streamed textures allocate VRAM only for mapped mip levels, integrated with `placement.rs`, `evict_unused`, `host_writes`, and `residency`
  - **Modularization & `lower.rs` size budget (`b2fb0ca`):** extracted `lower_vega.rs` and `lower_tests.rs` from `lower.rs` to stay under the 512 KiB per-file policy cap
  - **Divergent `wave32_in_64` & emulated mesh fixes (`59ffeb7`, `e9ab6c7`):** fixed `EXEC` guard preservation on `v_readfirstlane_b32`, divergent `wave32_in_64` intrinsics, emulated mesh blend state, and D3D12 heap/topology issues
  - **GCN 5 structured-buffer raw unification (`8404297`):** fixed the black 3D world bug on RX Vega 7 caused by `V#.NUM_RECORDS` element-vs-byte mismatch when accessing structured buffers via raw DXIL ops (`lower_vega.rs`, `lower.rs`, `descriptor/host.rs`)
  - **Translate & FPS optimization batch (`08949a9`):** merged `#2662` (`O(|known|)` BDD), `#2664` (zero-alloc IR/fields), `#2665` (cached ops & zero-copy bitcode), `#2958` (wave-coalesced write/read marks in `emit/lower/marks.rs`), `#2051` (multi-threaded `Batch::flush` in `copy_pool.rs`), and `#2848` (pooled copy-back buffers, lockless `pending_len`, stack `user_data`). See [`progress.md`](progress.md) and [`pull.md`](pull.md)
  - **Wave64-only PS mark-coalescing crash fix (`56b6b3e`):** disabled PS `mark_lead` wave-coalescing on Wave64-only hosts (`emit/lower/marks.rs`) to fix the `amdxc64.dll+0xd618a4` / `ntdll.dll+0x88f9` driver compiler crash on `c0a7fe77b1862e10`
  - **Resolution scaling (`1ecf41a`):** ported `VICTORY_OUTPUT` and `VICTORY_RENDER_SCALE` (`crates/victory-loader/src/resolution.rs`, `apps/victory/src/run.rs`) on top of the Series S `UPSCALER_6` patch
  - **DXIL optimizer (`d0c9123`):** ported `VICTORY_SHADER_OPT` (`crates/victory-shader/src/emit/bitcode/opt.rs`, default `31`) with the `f962387` `dx.op.getDimensions` `REUSE` guard, keyed into `shader_cache.rs` and `pipeline/library.rs`
  - **In-place 32-bit constants heap (`a5fe216`):** ported `VICTORY_PORT_CONSTANTS=1` (`inplace.rs`, `port.rs`, `shadow.rs`, `descriptor/host.rs`, `watch.rs`, `host.rs`, `draw.rs`, `xmem.rs`), mapping the constants/skinning heap directly via `ID3D12Device13::OpenExistingHeapFromAddress1` (verified `S_OK` on RX Vega 7)
  - **Raw root CBV reads (`01a2f20`):** ported `VICTORY_PORT_CBV=1` (`port_cbv.rs`, `emit/shader.rs`, `lower_vega.rs`, `lower.rs`, `descriptor/host.rs`, `draw.rs`, `shader_cache.rs`, `pipeline/library.rs`) lowering root CBVs to direct `t<slot>` root SRVs with 16-byte raw SRV/UAV alignment guards
  - **Scene-derived ambient light stand-in (`d5ce020`, upgraded from `trustno1-coder/VIctory-NVIDIA`):** ported `e5462b4` / `bdc9fb0` from `kalinizer/victory-polaris` (`ambient.rs`, `draw.rs`, `pipeline.rs`), computing 64×36 radiance cells + 160×90 depth-aware spherical harmonics from `CS_CacheScreenSpaceRadiance` with AMD magenta + blue-violet probe filtering (`VICTORY_AMBIENT`, `VICTORY_AMBIENT_SHADE`, `VICTORY_AMBIENT_SETTLE`, `VICTORY_AMBIENT_AIR`), plus `trustno1-coder/VIctory-NVIDIA` fixes (`bda6e59`, `d226814`, `826ba15`, `2db59d4`, `f89b79d`, `9b91d27`, `9f6fb77`, `8cf0a75`): placed-texture unwrapping in `draw.rs::ambient`, stencil-plane depth exclusion + fallback scans, `Role::Miss` specular stand-in (`VICTORY_AMBIENT_SPECULAR`, `VICTORY_AMBIENT_SPECULAR_ALPHA`), and `VICTORY_AMBIENT_ONCE=1` one-bounce tracking

## 3. Repository Rules & Safety Boundaries

1. **Source-only repository & file size cap (`docs/asset-policy.md`):** Never commit game dumps, `.exe`/`.dll`/`.rpf` files, extracted shaders, captures, or logs. Only `crates/victory-formats/src/bcpack-tables.bin` is exempted in `xtask/src/policy.rs`. Note also the **512 KiB (`524,288` byte) per-file limit**: `crates/victory-shader/src/emit/lower.rs` sits around `509,028` bytes, so place new emitter helpers in `lower_vega.rs` or `emit/lower/*.rs` submodules and always verify before committing:
   ```powershell
   cargo xtask policy --staged
   ```
2. **No DRM/auth circumvention:** User-supplied titles must be legally obtained and live outside the repository. Do not store personal machine paths in committed docs.
3. **Shader & pipeline caches:** `emit::host_caps_key()`, `emit::shader_opt()` (`VICTORY_SHADER_OPT`), and `emit::constants_raw()` (`VICTORY_PORT_CBV=1`) are hashed into `shader_cache.rs` and `pipeline/library.rs`, so Vega-lowered DXIL never collides with RDNA2/WARP caches and `VICTORY_PORT_CBV=1` keeps separate entries from default caches. Any edit to `crates/victory-shader` updates `SOURCE_HASH` and invalidates the cache, so re-run `victory translate --cache --pipelines --jobs 8` (setting `$env:VICTORY_PORT_CBV = '1'` when running with `VICTORY_PORT_CBV=1`) on the target Vega GPU after emitter changes.
4. **Unsafe, lints, & GPU tests:** Workspace lints deny `unsafe_op_in_unsafe_fn` and `unused_must_use`, and warn on `undocumented_unsafe_blocks`. Keep `// SAFETY:` comments accurate, run clippy with `-D warnings`, and always run GPU test suites single-threaded (`-- --test-threads=1`) to avoid concurrent D3D12 device contention. Also use PowerShell 5.1 syntax (`;` + `$LASTEXITCODE`, never `&&`).

## 4. Build, Test, and Verification Commands

```powershell
# Policy & lint checks
cargo xtask policy --staged
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Unit + Vega GPU hardware tests (always --test-threads=1 for GPU suites)
cargo test -p victory-shader --lib --locked -- --test-threads=1
cargo test -p victory-gpu --lib --locked -- --test-threads=1
cargo test --workspace --locked

# Build fastest runtime binary (release) + profiling/keys binary (diag)
cargo build --release -p victory --locked
cargo build --profile diag -p victory --features diagnostics,victory-gpu/diagnostics --locked
```

### Recommended Vega iGPU Runtime Environment (`--console series-s`)

```powershell
$env:VICTORY_MESH_EMULATION      = '1'
$env:VICTORY_MESH_BUFFER_MB      = '16'
$env:VICTORY_LEVELS_ON_DEMAND    = '1'
$env:VICTORY_PORT_CONSTANTS      = '1'
$env:VICTORY_PORT_CBV            = '1'
$env:VICTORY_OUTPUT              = '1280x720'
$env:VICTORY_RENDER_SCALE        = '0.67'
$env:VICTORY_CARD_BUFFERS_MIN_KB = '4'
$env:VICTORY_CARD_BUFFERS_MB     = '4096'
$env:VICTORY_SKIP_PIPELINES      = '39f0e9aeb156a61c'
$env:VICTORY_AMBIENT             = '0.6'
$env:VICTORY_LOG                 = 'warn'
```

*(Also set `$env:VICTORY_PORT_CBV = '1'` when running `victory translate --cache --pipelines` so cached shaders/PSOs match raw root CBV lowering.)*

Use `target\release\victory.exe` for best gameplay FPS (leave `VICTORY_CAPTURE`, `VICTORY_DUMP_TARGETS`, and `VICTORY_GPU_TIMING` unset). Use `target\diag\victory.exe` when `VICTORY_KEYS`, `fps`/`copystat` logging, or `VICTORY_D3D12_DEBUG=1` + `VICTORY_D3D12_DRED=1` are needed.

## 5. Known Vega Hardware Caveats & Test Status

- **Verified passing on RX Vega 7 (`d5ce020` + ambient fix):** `emit::lower::tests` (34/34), `emit::bitcode::opt` (6/6), `victory-gpu --lib` (262/262, 4 ignored — including all 5 `ambient::tests::*`, `inplace::tests`, `port::tests`, `port_cbv::tests`, and `shader_cache::tests`), `victory-hle --lib` (153/153), and RX Vega 7 `OpenExistingHeapFromAddress` / `OpenExistingHeapFromAddress1` + `CreatePlacedResource(ALLOW_CROSS_ADAPTER | ALLOW_UNORDERED_ACCESS)` probe (`S_OK`). Prior `08949a9` full baseline also passed `stride_tests` (9/9), `waterfall_tests` (5/5), `wave_coalesced_marks_equal_per_lane_marks_on_the_gpu` (RX Vega 7 & WARP), `structure::tests`, `ir::tests`, and `bitcode::tests`.
- **Pre-existing GCN 5 test failures in full `victory-shader` suite (unrelated to `8404297`/`08949a9`):** `image_atomics_of_64_bits` (element 0 umax), `gathers_at_a_level_match_the_hardware` (lane 2), `title_memory_is_read_and_written_at_computed_addresses` (last 2 descriptor words), plus tests missing `TEST_MIN_WAVE`.
- **Custom `attributeAtVertex` / weighted barycentric shaders:** Unsupported on hosts without `BarycentricsSupported` and rejected cleanly with an explicit error instead of emitting invalid DXIL.
- **Mesh shaders with cross-lane ops:** Still rejected in `lower.rs::cross_lane` (SM 6.6 `WaveSize` does not apply to mesh stages, and emulated mesh CS does not yet emulate cross-lane ops via LDS).
