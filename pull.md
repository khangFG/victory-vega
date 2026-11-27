# Notable Upstream Updates & Pull Requests (`Marvisak/VIctory`)

- **Upstream repository:** https://github.com/Marvisak/VIctory
- **`VIctory-Vega` base commit (via `victory-polaris`):** `940d32f` (2026-11-25)
- **Upstream `main` HEAD synced:** `63bcca4` (2026-11-25) — **150 commits merged (0 commits behind)**
- **Target hardware profile:** AMD Radeon RX Vega (GCN 5, Wave64-only, non-mesh, no native `SV_Barycentrics`, low VRAM / 16 GB system RAM)

---

## 1. Upstream `main` Commits Synced (`940d32f..63bcca4`)

Upstream `main` (`940d32f..63bcca4`, 150 commits, now merged) includes major VRAM, staging, watch, and shader-correctness improvements:

### Texture, VRAM, Staging & Watch Architecture
- `a7381ed` — *GPU: host textures made on first use* (avoids allocating host copies until referenced)
- `fdf34ab` & `8a3d975` — Shared staging pool and large carved upload buffers instead of one staging buffer per texture
- `cc5f961` — *GPU: a texture re-detiles only the subresources whose pages changed*
- `69d1e8e` & `11e9a54` — Placed texture heaps (`placement.rs`) + VRAM-budget LRU texture eviction (`evict_unused`)
- `4959d34`, `52d14b1`, `cea278c`, `ab2e2b2`, `56f75f8` — Lazy bindless texture creation & refresh outside heap locks
- `f68442f` — *GPU: a texture a host copy writes is the host's from the copy's submission* (deferred `host_writes` ordering)
- `8f2a6f1` — Per-thread staging read scratch buffer
- `ea81289` — Batched submission texture upload barriers
- `2d4aa55` — *GPU: the snapshot pool keeps at most 512 MiB*
- `280b150` & `cc0e153` — Faster region-by-region change tracking with a dense dirty bitmap
- `136e39d` (#2455), `106377d` (#2377), `384b0c0` (#2239), `5312628` (#2234) — Write-watch, partial-refresh, alias-page, and released view-heap page-table fixes

### Shader, AMD & GPU Correctness
- `0665a0e` (#573) — *Shaders: scope wave64 `ds_bpermute_b32` to 32-lane halves*
- `cc7ec2a` (#1599) — *Shaders: 64-bit integer compares on their dwords (AMD orders a shared-dword i64 compare wrong)*
- `e04c078` (#2143) — *Shaders: a program's constant table starts after its instructions (GBs of NVIDIA video memory)*
- `286a00e` — *Shaders: the DXIL writer leaves out what no edge or reader reaches*
- `ea488a9` (#2462) — *GPU: no typed unordered view in a format the host cannot store to*
- `366a978` (#2468) — *Shaders: guarded scalar writes kill stale SGPRs in `dead_on_edge`*
- `120233c` (#2448) — *Shader: ignore guarded or modified `v_mov_b32` in `Lower::konst`*
- `e815cef` (#2484) — *shader: look up last byte for straddling memory writes*
- `f3578e3` (#2493) — *Shaders: decode f16 inline float constants in whole-wave `v_add_f16` simulation*
- `842b0cb` (#158) — *GPU: the depth bounds test reaches the host*

---

## 2. Notable Open Pull Requests

### Merged into `VIctory-Vega`
| PR | Title | Status | Notes |
| --- | --- | --- | --- |
| [#2389](https://github.com/Marvisak/VIctory/pull/2389) | GPU: streamed textures take video memory only for the levels the title holds (`VICTORY_LEVELS_ON_DEMAND=1`) | **Merged natively (atop `63bcca4`)** | Uses `CreateReservedResource`/`CreateReservedResource2` + `UpdateTileMappings` on Tiled Resources Tier 2+ GPUs atop `placement.rs`, `evict_unused`, `Scratch`, `host_writes`, and `residency`; verified on local RX Vega. |
| [#2662](https://github.com/Marvisak/VIctory/pull/2662) | Shaders: O(|known|) BDD `forced()` sweep, read-only `implies()`, per-block reuse in `finish()`, FxHash in `meet_in()` | **Merged** | Eliminates quadratic BDD blowup during structurisation (93×–1,213× faster BDD on complex shaders). |
| [#2664](https://github.com/Marvisak/VIctory/pull/2664) | Shaders/IR: eliminate per-instruction/CFG `Vec` allocations and O(1) `effects` dedup | **Merged** | Zero-alloc `valu_dwords`, stack `Exit::succs()`, bit-scanning `RegSet::members()`, O(1) `effects()` dedup. |
| [#2665](https://github.com/Marvisak/VIctory/pull/2665) | Shaders/Bitcode: cache `Shader::op` declarations, primitive types, scalar constants, and zero-copy bitcode `Writer` | **Merged** | Cuts DXIL emission allocations and emits bit-identical bitcode faster. |
| [#2958](https://github.com/Marvisak/VIctory/pull/2958) | Shaders: one lane of a wave sets a mark its wave shares | **Merged** | Coalesces per-lane write/read-mark bitmap atomics to one leading lane per wave in CS/VS/PS (`emit/lower/marks.rs`). |
| [#2051](https://github.com/Marvisak/VIctory/pull/2051) | GPU: a submission's refreshes copy on several threads | **Merged** | Multi-threaded `Batch::flush` copies via `copy_pool` workers (`>4 MiB` batches). |
| [#2848](https://github.com/Marvisak/VIctory/pull/2848) | victory-gpu: take per-frame allocations, locks and system calls off the frame paths | **Merged** | Pooled copy-back read buffers (`Back`), lockless atomic `pending_len` fast path in `Shadow::update`, stack-allocated `[u32; 64]` user data in `draw.rs`. |

### High Priority: VRAM & System Commit Budget
| PR | Title | Label / State | Vega Notes |
| --- | --- | --- | --- |
| [#3444](https://github.com/Marvisak/VIctory/pull/3444) | GPU: evict LRU title-backed textures under Windows system commit pressure | Open | Evicts LRU textures when `CommitTotal > 0.85 * CommitLimit` (`K32GetPerformanceInfo`) + optional `VICTORY_TEXTURE_BUDGET_MB`. Directly prevents `ERROR_COMMITMENT_LIMIT` on 16 GB RAM hosts. |
| [#3438](https://github.com/Marvisak/VIctory/pull/3438) | HLE: add opt-in demand-commit for large `XMemVirtualAlloc` blocks (`VICTORY_XMEM_LAZY_COMMIT`) | Open | Avoids committing untouched 4+ MiB allocations (~76% untouched in testing) via VEH first-touch commit. |
| [#3443](https://github.com/Marvisak/VIctory/pull/3443) | GPU: pool list upload and readback staging buffers with bounded idle cap | Open | Pools <=16 MiB list upload/readback buffers with a 32 MiB idle cap instead of per-copy allocations. |
| [#3447](https://github.com/Marvisak/VIctory/pull/3447) | feat(gpu): add adaptive DXGI and system RAM budget scaling across tiers | Open | Classifies Low / Balanced / Full memory tiers with hysteresis. |
| [#2112](https://github.com/Marvisak/VIctory/pull/2112) | GPU: title memory in place instead of copied (`VICTORY_IN_PLACE=1`) | Draft | Uses `OpenExistingHeapFromAddress` to cut loose-window host copies drastically; untested on Windows/AMD. |

### High Priority: Shader Correctness & Vega Compatibility
| PR | Title | Label / State | Vega Notes |
| --- | --- | --- | --- |
| [#2698](https://github.com/Marvisak/VIctory/pull/2698) | Shaders: derive `ATOMIC64_TYPED` and `ATOMIC64_HEAP` from the atomics that are written | `triage: merge candidate` | Stops dead/raw atomics from spuriously requesting `AtomicInt64OnTypedResource`. |
| [#1065](https://github.com/Marvisak/VIctory/pull/1065) | Shaders: use validator 1.8 for compute derivatives | Open | Bumps `dx.valver` 1.8 / `PSV0` v3 only for compute shaders with live gradient ops (unblocks 24+ compute shaders). |
| [#2091](https://github.com/Marvisak/VIctory/pull/2091) | Shaders: lower Wave32 compute branches that reuse lane masks as scalar data | Open | Lowers 40+ Wave32 compute shaders that reuse lane masks as scalar data. *(Check `EXEC` interaction with our `wave32_in_64` lowering.)* |
| [#2312](https://github.com/Marvisak/VIctory/pull/2312) | GPU, Shaders: spread compute dispatches past 65,535 groups in x across z | Open | Prevents D3D12 dispatch drops when `x > 65535`. |
| [#2657](https://github.com/Marvisak/VIctory/pull/2657) | Shaders: scope DXIL bitcode liveness to reachable blocks and live phi edges | `triage: merge candidate` | Fixes DXIL bitcode liveness accuracy across reachable blocks/phis. |
| [#2622](https://github.com/Marvisak/VIctory/pull/2622) | Shaders: a constant branch through a folded block drops its untaken phi entry | `triage: merge candidate` | Fixes constant-folded block phi lowering. |
| [#2480](https://github.com/Marvisak/VIctory/pull/2480) | Shaders: BDD meet rejects a node held both ways and takes one condition as met | `triage: merge candidate` | Fixes BDD condition solver bug. |
| [#385](https://github.com/Marvisak/VIctory/pull/385) | Shaders: track every incoming edge of a lane-mask phi | `triage: merge candidate` | Fixes multi-edge lane-mask phi tracking. |
| [#223](https://github.com/Marvisak/VIctory/pull/223) | Shaders: translate for the host's wave sizes | `triage: merge candidate` | Upstream `emit::Target` wave-range + cache-key plumbing (overlaps our `HOST_MIN_WAVE` / `host_caps_key()`). |

### Frame-Time & Load-Time Performance
| PR | Title | Label / State | Vega Notes |
| --- | --- | --- | --- |
| [#2985](https://github.com/Marvisak/VIctory/pull/2985) | GPU: a tracked texture write detiles and copies only the rows it touches | `triage: merge candidate` | Reduces partial-write texture detiling/copy overhead. |
| [#1653](https://github.com/Marvisak/VIctory/pull/1653) | victory-gpu: split the pipeline library into shards so a save writes only what changed | Open | Splits 600+ MB PSO library into 16 shards for faster periodic saves. |

### Skip for RX Vega (Wave32-Host, NVIDIA, or RDNA2+ Only)
- **Wave64-on-Wave32 emulation:** [#189](https://github.com/Marvisak/VIctory/pull/189), [#233](https://github.com/Marvisak/VIctory/pull/233), [#439](https://github.com/Marvisak/VIctory/pull/439), [#2633](https://github.com/Marvisak/VIctory/pull/2633), [#578](https://github.com/Marvisak/VIctory/pull/578) — unneeded because RX Vega is natively Wave64.
- **NVIDIA descriptor-heap caps & raw-view workarounds:** [#2121](https://github.com/Marvisak/VIctory/pull/2121), [#1777](https://github.com/Marvisak/VIctory/pull/1777), [#1548](https://github.com/Marvisak/VIctory/pull/1548), [#2885](https://github.com/Marvisak/VIctory/pull/2885) — unneeded because Vega supports D3D12 Resource Binding Tier 3 without NVIDIA's 1M heap cap.
- **RDNA 2+ hardware features:** [#3335](https://github.com/Marvisak/VIctory/pull/3335) (`SV_Barycentrics`), [#3294](https://github.com/Marvisak/VIctory/pull/3294) (`VICTORY_HW_BVH=1`).
