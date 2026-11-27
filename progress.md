# VIctory-Vega Progress & Chat Handoff

- **Branch / HEAD:** `main` @ `08949a9` (synced atop `Marvisak/VIctory` `63bcca4`)
- **Target Hardware:** AMD Radeon RX Vega 7 (GCN 5 iGPU, Wave64-only `min_wave = 64`, SM 6.6, `MeshMode::Emulate`, `BarycentricsSupported = false`)
- **Active Profile:** `--console series-s`
- **Built Binaries Ready:** `target\release\victory.exe` (max speed) and `target\diag\victory.exe` (`VICTORY_KEYS` / `copystat` / `capture`)

---

## 1. Current Milestone Summary

The **3D world loads and renders on AMD RX Vega 7 (GCN 5)** with only minimal texture quirks remaining. Both shader translation throughput and CPU/GPU frame hot paths have now been optimized in `08949a9` and are ready for live cache warm-up and FPS benchmarking.

### Completed Commits Since `victory-polaris` / Upstream Sync (`63bcca4`)

| Commit | Change | Impact |
| --- | --- | --- |
| `e08d1e3` | Merged upstream `Marvisak/VIctory` `main` (`63bcca4`, 150 commits) | Placed texture heaps, VRAM LRU eviction, lazy bindless creation, shared staging pool, 512 MiB snapshot cap, wave64 `ds_bpermute_b32` half scoping |
| `b710808` | Merged upstream PR `#2389` natively atop `63bcca4` | Added `VICTORY_LEVELS_ON_DEMAND=1` (saves ~`1,026 MB` VRAM in live world frames via reserved tiled mips) |
| `b2fb0ca` | Split `lower_tests.rs` & `lower_vega.rs` out of `lower.rs` | Kept `lower.rs` under the 512 KiB (`524,288` B) asset-policy ceiling |
| `59ffeb7`, `e9ab6c7` | Fixed `wave32_in_64` `EXEC` guard on `v_readfirstlane_b32`, divergent intrinsics, emulated mesh blend state, D3D12 heap/topology errors | Fixed game crashes during 3D scene bring-up |
| `8404297` | **Root-caused & fixed black 3D world on GCN 5:** unified 4-byte-aligned `StructuredBuffer` accesses & host SRV/UAV views onto raw `R32_TYPELESS` + `FLAG_RAW` with 16-byte `view_copy_align` (`lower_vega.rs`, `lower.rs`, `descriptor/host.rs`) | On GCN 5, D3D12 `StructuredBuffer` sets `V#.NUM_RECORDS = NumElements` (unlike bytes on RDNA 2+), so `rawBufferLoad`/`rawBufferStore` failed bounds checks and zeroed `LuxCompressionInfo` / lighting / sky / composite |
| `08949a9` | **Merged 6 upstream translate & FPS PRs:** `#2662` (`O(|known|)` BDD), `#2664` (zero-alloc IR/fields), `#2665` (cached ops & zero-copy bitcode), `#2958` (wave-coalesced write/read marks), `#2051` (multi-threaded `Batch::flush` copies), `#2848` (pooled copy-back `Back`, lockless `pending_len`, stack `user_data`) | Directly targets cold shader translation latency, Wave64 mark-bitmap atomic traffic, CPU buffer-refresh time, and copy-back page faults |

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

Location: `%LOCALAPPDATA%\victory\data\<title_id>\storage\settings\settings.xml` (not edited by agent — back up before changing). Current XML renders a 4K UI (`3840x2160`) and 1080p scene onto a `1365x768` panel with RT dynamic shadows and SSR enabled:

| Node | Current | Suggested for Vega | Expected Effect |
| --- | --- | --- | --- |
| `<uiResolution>` | `3840 × 2160` | `1280 × 720` (or `1920 × 1080`) | Eliminates 4K UI PresentX surface overhead |
| `<resolutionFullscreen>` | `1920 × 1080` | `1280 × 720` | ~55% fewer pixels shaded & copied |
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

## 4. Verification Snapshot at `08949a9`

- `victory-shader` targeted suite (`wave_coalesced_marks_equal_per_lane_marks_on_the_gpu`, `writes_through_views_mark_their_pages`, `straddling_writes`, `structure::tests`, `ir::tests`, `bitcode::tests`, `stride_tests`, `waterfall_tests`, `lower::tests` on RX Vega 7 + WARP): **91 passed, 0 failed** (3 benchmarks ignored).
- `victory-gpu --lib`: **248 passed, 0 failed** (4 ignored), including `copy_pool::tests` and `queued_partial_refresh_under_marked_range_updates_saved_bytes`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` (default & `--all-features`): **clean**.
- `target\release\victory.exe translate --validate` on test DXBC fixtures: **3/3 emitted & validated**.
- `crates/victory-shader/src/emit/lower.rs` size: **`509,538` bytes** (policy limit: `524,288` bytes).

---

## 5. Next Steps for the New Chat

1. **Warm caches & measure post-`08949a9` baseline:**
   - Run `target\release\victory.exe translate --cache --pipelines --jobs 8 ...` to pre-translate shaders and build PSOs for `08949a9`.
   - Do a clean run (with `VICTORY_CAPTURE` / `VICTORY_DUMP_TARGETS` disabled) using either `release` (max FPS) or `diag` (to compare `fps` / `copystat` against Run #3), and test the recommended `settings.xml` edits.
2. **Remaining high-ROI CPU/FPS improvements (not yet merged):**
   - Complete the remaining parts of upstream PR `#2848` if beneficial: `SHADER_HEAPS` `RwLock` (addressed `243.6 ms/frame` title-thread waits in Run #3), chunked stack `HostHeap::sync_slots` (`57.3 ms/frame`), and batched `VirtualProtect` page opening in `watch.rs` (`67.05 ms/frame`).
   - Consider `#2985` (row-scoped texture detile/copy), `#1653` (sharded PSO library saves), and commit-pressure helpers (`#3444`, `#3443`, `#3438`) from [pull.md](pull.md).
3. **Visual / shader polish:**
   - Investigate the remaining minimal texture quirks reported on RX Vega 7 and check why `pixel 98bd9f9ae5f9c7e3` (~4.9 draws/frame) and the ~3.5/frame compute dispatches are dropped (compare against upstream shader PRs `#2698`, `#1065`, `#2091`, `#2312` in [pull.md](pull.md)).
