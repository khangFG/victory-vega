# VIctory-Vega Progress & Chat Handoff

- **Branch / HEAD:** `main` (`01a2f20`, synced atop `Marvisak/VIctory` `63bcca4`; includes `08949a9` perf batch, `56b6b3e` PS mark-coalescing fix, `1ecf41a` resolution scaling, `d0c9123` DXIL optimizer, `a5fe216` in-place constants heap, and `01a2f20` raw root CBV reads)
- **Target Hardware:** AMD Radeon RX Vega 7 (GCN 5 iGPU, Wave64-only `min_wave = 64`, SM 6.6, `MeshMode::Emulate`, `BarycentricsSupported = false`)
- **Active Profile:** `--console series-s`
- **Built Binaries Ready:** `target\release\victory.exe` (max speed) and `target\diag\victory.exe` (`VICTORY_KEYS` / `copystat` / `capture`)

---

## 1. Current Milestone Summary

The **3D world loads and renders on AMD RX Vega 7 (GCN 5)** with only minimal texture quirks remaining. All **Tier 1** AMD fork harvests are now ported, verified on RX Vega 7, and committed (`1ecf41a`, `d0c9123`, `a5fe216`, `01a2f20`), alongside the `56b6b3e` `amdxc64` PS pipeline-compile crash fix. Caches are ready to be warmed at `01a2f20` for live Vega benchmarking.

### Completed Milestones Since `victory-polaris` / Upstream Sync (`63bcca4`)

| Commit | Change | Impact |
| --- | --- | --- |
| `e08d1e3` | Merged upstream `Marvisak/VIctory` `main` (`63bcca4`, 150 commits) | Placed texture heaps, VRAM LRU eviction, lazy bindless creation, shared staging pool, 512 MiB snapshot cap, wave64 `ds_bpermute_b32` half scoping |
| `b710808` | Merged upstream PR `#2389` natively atop `63bcca4` | Added `VICTORY_LEVELS_ON_DEMAND=1` (saves ~`1,026 MB` VRAM in live world frames via reserved tiled mips) |
| `b2fb0ca` | Split `lower_tests.rs` & `lower_vega.rs` out of `lower.rs` | Kept `lower.rs` under the 512 KiB (`524,288` B) asset-policy ceiling |
| `59ffeb7`, `e9ab6c7` | Fixed `wave32_in_64` `EXEC` guard on `v_readfirstlane_b32`, divergent intrinsics, emulated mesh blend state, D3D12 heap/topology errors | Fixed game crashes during 3D scene bring-up |
| `8404297` | **Root-caused & fixed black 3D world on GCN 5:** unified 4-byte-aligned `StructuredBuffer` accesses & host SRV/UAV views onto raw `R32_TYPELESS` + `FLAG_RAW` with 16-byte `view_copy_align` (`lower_vega.rs`, `lower.rs`, `descriptor/host.rs`) | On GCN 5, D3D12 `StructuredBuffer` sets `V#.NUM_RECORDS = NumElements` (unlike bytes on RDNA 2+), so `rawBufferLoad`/`rawBufferStore` failed bounds checks and zeroed `LuxCompressionInfo` / lighting / sky / composite |
| `08949a9` | **Merged 6 upstream translate & FPS PRs:** `#2662` (`O(\|known\|)` BDD), `#2664` (zero-alloc IR/fields), `#2665` (cached ops & zero-copy bitcode), `#2958` (wave-coalesced write/read marks), `#2051` (multi-threaded `Batch::flush` copies), `#2848` (pooled copy-back `Back`, lockless `pending_len`, stack `user_data`) | Directly targets cold shader translation latency, Wave64 mark-bitmap atomic traffic, CPU buffer-refresh time, and copy-back page faults |
| `56b6b3e` | **Disabled PS `mark_lead` wave-coalescing on Wave64-only hosts** (`crates/victory-shader/src/emit/lower/marks.rs`) | Fixed `amdxc64.dll+0xd618a4` / `ntdll.dll+0x88f9` driver compiler access violation on `c0a7fe77b1862e10` introduced by `#2958` when compiling PS pipelines on RX Vega 7 |
| `1ecf41a` | **Ported `VICTORY_OUTPUT` + `VICTORY_RENDER_SCALE`** (`crates/victory-loader/src/resolution.rs`, `apps/victory/src/run.rs`, from `avatarmanz/VIctory-7900XTX` `f63b4b1`) | `settings.xml` alone cannot shrink output size without hitting a hardcoded back-buffer pool `int3` check (`+0x247af25`); unlocks `VICTORY_OUTPUT=1280x720` and `VICTORY_RENDER_SCALE=0.67` or `0.5` cleanly with our Series S `UPSCALER_6` patch (cutting Vega GPU pixel/bandwidth load ~2.25×–4×) |
| `d0c9123` | **Ported Tier 1A: DXIL Optimizer Pass (`VICTORY_SHADER_OPT`, default `31`)** (`crates/victory-shader/src/emit/bitcode/opt.rs` + `f962387` `getDimensions` guard; wired in `emit/mod.rs`, `emit/shader.rs`, and keyed in `shader_cache.rs` & `pipeline/library.rs`) | Runs 5 exact cleanup passes (`DEAD`, `FOLD`, `REUSE`, `FACTS`, `LOADS`) before DXIL emission to reduce VGPR pressure and `amdxc64` PSO compile times |
| `a5fe216` | **Ported Tier 1B: In-Place 32-Bit Constants Heap (`VICTORY_PORT_CONSTANTS=1`, `VICTORY_INPLACE`, `VICTORY_INPLACE_PROBE`)** (`inplace.rs`, `port.rs`, `shadow.rs`, `descriptor/host.rs`, `watch.rs`, `host.rs`, `draw.rs`, `crates/victory-hle/src/modules/xmem.rs`) | Maps the title's 32-bit constants/skinning heap directly via `ID3D12Device13::OpenExistingHeapFromAddress1` on a duplicate section view (verified `S_OK` on RX Vega 7 hardware with `ALLOW_CROSS_ADAPTER \| ALLOW_UNORDERED_ACCESS`), eliminating host copies, write-watch tracking, and copy-backs for that heap; `draw.rs` already clamps every indirect call individually with `uav_barrier`s (inherently satisfying `VICTORY_PORT_CONSTANTS_ORDER`) |
| `01a2f20` | **Ported Tier 1C: Raw Root CBV Reads (`VICTORY_PORT_CBV=1` + 16-byte raw SRV/UAV alignment guard)** (`port_cbv.rs`, `emit/shader.rs`, `emit/mod.rs`, `lower_vega.rs`, `lower.rs`, `lower_tests.rs`, `descriptor/host.rs`, `draw.rs`, `host.rs`, `shader_cache.rs`, `pipeline/library.rs`) | Table and resource-heap CBVs in `VIctory-Vega` already lowered to raw `R32_TYPELESS` SRVs; `VICTORY_PORT_CBV=1` extends raw lowering to root CBVs (direct `t<slot>` root SRVs in `ROOT_CBV_SPACE`), eliminating 64 KiB scratch upload copies on non-256B-aligned root CBVs while keeping default cache keys unchanged when disabled |

*(See [pull.md](pull.md) for full merged/open upstream PR tracking.)*

---

## 2. Pre-`08949a9` Live Profile Baseline (Run #3)

Measured in `run-debug-3.log` (`diag` build with capture/dump enabled, `--console series-s`, Series X-patched upscaler `6`, `1920x1080` fullscreen + `3840x2160` `uiResolution` fitted to a `1365x768` display):

- **World frames:** `0.8–1.0 fps` (`1,014.5–1,545.5 ms/frame`, `3,548–10,880` draws/frame).
  - Typical `1.0 fps` (`1,014.5 ms/frame`) breakdown:
    - **GPU:** `633.4 ms` (`619.2–810.5 ms` range)
    - **CPU Preparing:** `191.4 ms` (`50.6–98.1 ms` buffer refresh copies across `19.4 GB/frame` refreshed / `284.1 MB` snapshots, `57.3 ms` mirroring `497,664` heap slots, `67.05 ms` guard handler with `4,144` `VirtualProtect` calls, `243.6 ms` title threads waiting on view locks)
    - **CPU Copy-Back:** `65.0 ms` (`50.3 ms` marked pages written back, `1,841` page faults/frame)
- **Translation & PSO Compilation (cold cache because `8404297` updated the translator key):**
  - `3,030` runtime translations: `668.1 s` total CPU time (avg `220.5 ms`, max `18,065 ms`; `267` shaders `>= 500 ms`).
  - `645` host PSO compiles: `1,048.2 s` total CPU time (avg `1,625.2 ms`, max `16,419 ms`).
- **On-Demand Mip Tiles (`VICTORY_LEVELS_ON_DEMAND=1`):**
  - `781` alive tiled textures, `586 MB` committed to `1,703` live levels vs `1,612 MB` full (`1,026 MB` VRAM saved).
- **Remaining Dropped Draws/Dispatches:**
  - `dropped 4.9/frame without a host pipeline: pixel 98bd9f9ae5f9c7e3`
  - Plus `~3.5/frame` compute dispatches (`a root view's offset: no root view, or outside title buffers`).

---

## 3. Recommended `settings.xml` Adjustments (User-Managed)

Location: `%LOCALAPPDATA%\victory\data\<title_id>\storage\settings\settings.xml` (backed up to `settings.xml.bak`). `<uiResolution>` (`3840 × 2160 → 1280 × 720`) and `<resolutionFullscreen>` (`1920 × 1080 → 1280 × 720`) have already been updated locally; keep `$env:VICTORY_OUTPUT = '1280x720'` set at launch so `POOL_CHECK` bypasses the `+0x247af25` `int3` check (`settings.xml` has no render-scale field, so `VICTORY_RENDER_SCALE=0.67` or `0.5` comes from the environment variable). Other `<video>` / lighting / LOD tweaks below remain optional:

| Node | Baseline | Suggested / Applied for Vega | Expected Effect |
| --- | --- | --- | --- |
| `<uiResolution>` | `3840 × 2160` | **`1280 × 720` (applied)** | Eliminates 4K UI PresentX surface overhead |
| `<resolutionFullscreen>` | `1920 × 1080` | **`1280 × 720` (applied)** | ~55% fewer pixels shaded & copied |
| `<rtDynamicShadows>` | `true` | `false` | Drops RT dynamic shadow trace/denoise work |
| `<shadowParticleShadows>` | `true` | `false` | Cuts particle shadow cost |
| `<waterReflectionSSR>` | `true` | `false` | Disables water SSR pass |
| `<waterReflectionResX/Y>` | `512` / `256` | `256` / `128` | Reduces water reflection target size |
| `<ssroSampleCount>` | `2` | `1` | Halves screen-space occlusion samples |
| `<motionBlur>` | `true` | `false` | Removes motion blur fullscreen pass |
| `<damageModelsEnabled>` | `true` | `false` | Optional extra CPU/GPU savings |
| `<sharpenEnabled>` | `true` | `false` | Optional minor post-pass savings |
| `<grassLod>` | `0.800000` | `0.400000` (or `0.250000`) | Cuts foliage draw load |
| `<lodScale>` | `1.000000` | `0.600000` | Cuts total draws/frame |
| `<manualMipBias>` | `-1.500000` | `0.000000` | Avoids forcing extra high-res mip pressure |
| `<desiredFPS>` / `<frameRateLimit>` | `30` / `30` | `60` / `60` | Uncaps 30 FPS ceiling when headroom exists |

*(Note: `--console series-s` and `KNOWN_BUILDS` upscaler patches may still override/clamp some video fields at runtime.)*

---

## 4. Verification Snapshot at `01a2f20`

- **RX Vega 7 hardware probe:** `OpenExistingHeapFromAddress`, `OpenExistingHeapFromAddress1` (`ID3D12Device13`), and `CreatePlacedResource(ALLOW_CROSS_ADAPTER | ALLOW_UNORDERED_ACCESS)` all returned `S_OK`.
- `victory-shader`: `emit::lower::tests` **34 passed, 0 failed** (includes `root_cbv_lowers_to_root_srv_when_constants_raw_is_on`) and `emit::bitcode::opt` **6 passed, 0 failed** (on RX Vega 7 + WARP; `--test-threads=1`).
- `victory-loader --lib`: **40 passed, 0 failed** (includes all 5 `resolution::tests::*` tests from `1ecf41a`).
- `victory-gpu --lib`: **258 passed, 0 failed, 4 ignored** (includes `inplace::tests` 7/7, `port::tests` 2/2, `port_cbv::tests` 1/1, `shader_cache::tests`).
- `victory-hle --lib`: **153 passed, 0 failed** (includes `arrange_orders_fresh_slices_in_low_reservation`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings` (default & `--all-features`): **clean**.
- `cargo xtask policy`: **clean** (`209 files ok`).
- `crates/victory-shader/src/emit/lower.rs` size: **`509,028` bytes** (policy limit: `524,288` bytes — shrank by 510 bytes in `01a2f20`).
- Binaries built cleanly at `01a2f20`: `target\release\victory.exe` and `target\diag\victory.exe`.

---

## 5. Next Steps for the New Chat

1. **Warm caches & measure post-`01a2f20` Vega baseline:**
   - Run `target\release\victory.exe translate --cache --pipelines --jobs 8 ...` **with `$env:VICTORY_PORT_CBV = '1'`** (alongside `$env:VICTORY_MESH_EMULATION = '1'` and default `VICTORY_SHADER_OPT=31`), since both `d0c9123` and `VICTORY_PORT_CBV=1` affect shader/pipeline cache keys.
   - Benchmark live with `VICTORY_PORT_CONSTANTS=1`, `VICTORY_PORT_CBV=1`, `VICTORY_OUTPUT=1280x720`, and `VICTORY_RENDER_SCALE=0.67` (with `VICTORY_CAPTURE` / `VICTORY_DUMP_TARGETS` disabled). Optionally set `VICTORY_INPLACE_PROBE=1` once to log driver probe lines, or run `target\diag\victory.exe` to compare `port::log` / `port_cbv::log` / `fps` / `copystat` against Run #3.
2. **Proceed to Tier 2 CPU prep & stability harvests (and remaining upstream PRs):**
   - `VICTORY_PORT_DESCRIPTORS=1` (`port_descriptors.rs`), `VICTORY_PORT_GUARD=1`, `VICTORY_PORT_JOURNAL=1`, and `DexNom` range-scoped texture staging lock (`5f4c3df`) + shader/texture fixes (`ddb7332`, `921d8d3`).
   - Also consider upstream PRs `#2985` (row-scoped texture detile/copy), `#1653` (sharded PSO library saves), and commit-pressure helpers (`#3444`, `#3443`, `#3438`) from [pull.md](pull.md).
3. **Visual / shader polish:**
   - Investigate the remaining minimal texture quirks reported on RX Vega 7 and check why `pixel 98bd9f9ae5f9c7e3` (~4.9 draws/frame) and the ~3.5/frame compute dispatches are dropped (compare against upstream shader PRs `#2698`, `#1065`, `#2091`, `#2312` in [pull.md](pull.md)).

---

## 6. AMD Fork Harvest Findings (`Belobug`, `avatarmanz`, `DexNom`)

Evaluated three active AMD community forks. Because all three target RDNA 2/3/4 (native mesh shaders, Wave32, `SV_Barycentrics`, 16–24 GB VRAM), we harvest individual self-contained modules into `VIctory-Vega` rather than rebasing:

- **Lineage overview:**
  - `Belobug/VIctory-Belobug-AMD` & `avatarmanz/VIctory-7900XTX` share the "Renderer Ports 1–7" + DXIL optimizer architecture (dropping RX 7900 XTX Series S frame times from ~`280 ms` to ~`45 ms` / `22 fps`).
  - `DexNom/VIctory` is a separate `940d32f`-based Radeon/RayTime fork focused on RT, FSR 4, streaming, and shader recovery.

### Tier 1 — High-Impact GPU & FPS Targets for RX Vega 7
1. **`VICTORY_OUTPUT` + `VICTORY_RENDER_SCALE` (`avatarmanz` `f63b4b1`, `crates/victory-loader/src/resolution.rs`) — PORTED & COMMITTED (`1ecf41a`):**
   - Exposes `VICTORY_OUTPUT=1280x720` (plus `POOL_CHECK` bypass over `+0x247af25` `int3`) and `VICTORY_RENDER_SCALE=0.5..1.0` directly on the title's upscaler (`+0x28f1e78`). Avoids overshooting a `1366×768` display and cuts Vega GPU pixel/bandwidth load substantially.
2. **DXIL Optimizer Pass (`Belobug` `crates/victory-shader/src/emit/bitcode/opt.rs` + `f962387`) — PORTED & COMMITTED (`d0c9123`):**
   - Runs 5 exact pre-bitcode cleanup passes (`DEAD`, `FOLD`, `REUSE`, `FACTS`, `LOADS`) before DXIL emission (`VICTORY_SHADER_OPT`, default `31`). Strips dead register dwords, redundant branch guards, duplicate descriptor handles, and redundant in-block loads.
   - Reduces shader VGPR pressure/GPU time and shortens `amdxc64` PSO compile times (`1,048 s` CPU time in Run #3). Includes `f962387` (keeps `dx.op.getDimensions` out of `REUSE` to prevent AMD compiler crashes). Lives in `bitcode/opt.rs` (does not grow `lower.rs`).
3. **In-Place Constants Heap (`VICTORY_PORT_CONSTANTS=1`) + Raw Root CBVs (`VICTORY_PORT_CBV=1`) (`avatarmanz`/`Belobug`: `inplace.rs`, `port.rs`, `port_cbv.rs`, `0dec5dc`) — PORTED & COMMITTED (`a5fe216`, `01a2f20`):**
   - Opens the title's 174 MB 32-bit constants/skinning heap (`0x34310000`) via `ID3D12Device13::OpenExistingHeapFromAddress1` on a duplicate section view so the GPU reads title RAM in place (zero copies/watch/copy-backs, ~`174 MB` saved). Direct RX Vega 7 hardware probe returned `S_OK` for `OpenExistingHeapFromAddress`, `OpenExistingHeapFromAddress1`, and `CreatePlacedResource(ALLOW_CROSS_ADAPTER | ALLOW_UNORDERED_ACCESS)`; `draw.rs` already clamps every indirect call individually with `uav_barrier`s (inherently satisfying `VICTORY_PORT_CONSTANTS_ORDER`).
   - Table and resource-heap CBVs in `VIctory-Vega` already lowered to raw `R32_TYPELESS` SRVs; `VICTORY_PORT_CBV=1` (`01a2f20`) extends raw lowering to root CBVs (direct `t<slot>` root SRVs in `ROOT_CBV_SPACE`) and enforces 16-byte raw SRV/UAV alignment, eliminating 64 KiB scratch upload copies on non-256B-aligned root CBVs while leaving default cache keys untouched when off.

### Tier 2 — CPU Prep & Polish Candidates
- **`VICTORY_PORT_DESCRIPTORS=1` (`port_descriptors.rs`):** Patches 6 title descriptor-store sites to dirty-mark slots directly instead of scanning every written heap page (`57.3 ms/frame` in Run #3).
- **`VICTORY_PORT_GUARD=1` & `VICTORY_PORT_JOURNAL=1`:** Shared write-guard lock with per-64 KiB block claims, parallel refresh planning, and 64-shard journal (`67.05 ms` guard handler + `243.6 ms` lock waits in Run #3).
- **`DexNom` range-scoped texture staging lock (`5f4c3df`):** Prevents texture staging from stalling behind unrelated memory remaps (`846 ms -> 77 µs` in `DexNom` benchmark), plus shader/texture fixes (`ddb7332` integer-target blend + dual static samplers, `921d8d3` quad sampling / sparse textures / VS signature ordering).
- **Frame cap unlock (`VICTORY_FRAME_INTERVAL_MS=16.667`)** and **FSR 3.1 fallback (`Belobug` `4dc700c`)** once FPS rises above 30 or if external FidelityFX DLLs are used.

### Skip for RX Vega 7
- **FSR 4.1.2 (`-Fsr4K`, `DexNom` `ab34a8b`, `Belobug` `cd8dfc0`):** RDNA 4 only.
- **Experimental RT (`DexNom` RayTime, `avatarmanz` `VICTORY_PORT_RT`):** Vega lacks hardware RT cores and is GPU-bound.

