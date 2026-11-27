# AGENTS.md — VIctory-Vega

Instructions for AI coding agents and developers working in `VIctory-Vega`.

## 1. Mission & Target Hardware Profile

`VIctory-Vega` is a research fork of VIctory combining `victory-polaris` (base runtime, mesh emulation, BCPack, offline pipeline prebuild, Series S patches) with key capabilities harvested from `VIctory-RDNA1-Hybrid` plus new **Wave32-on-Wave64 compute lowering** specifically for **AMD Radeon RX Vega (GCN 5)** GPUs — including low-VRAM Vega APUs (e.g. `0x15E7`, ~512 MiB–2 GiB dedicated VRAM) and discrete Vega 56/64.

| Capability | AMD Vega (GCN 5) | How `VIctory-Vega` handles it |
|---|---|---|
| Direct3D 12 Shader Model | SM 6.6 | Required; dynamic resources enabled |
| Hardware Mesh Shaders | None (`NotSupported`) | Compute + unpack-VS emulation (`mesh_emu.rs`, `MeshMode::Emulate`) |
| Native Wave Size (`WaveLaneCountMin..Max`) | **64..64 (Wave64-only)** | Console Wave32 compute shaders lower as two isolated 32-lane halves inside `WaveSize(64)` (`wave32_in_64` in `crates/victory-shader/src/emit/lower.rs`) |
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
  - **On-demand mip tile allocation (`tiles.rs`, upstream PR #2389 merged natively atop `63bcca4`):** opt-in `VICTORY_LEVELS_ON_DEMAND=1` uses D3D12 `CreateReservedResource` / `UpdateTileMappings` on Tiled Resources Tier 2+ GPUs so streamed textures allocate VRAM only for mapped mip levels, integrated with `placement.rs`, `evict_unused`, `host_writes`, and `residency`

## 3. Repository Rules & Safety Boundaries

1. **Source-only repository (`docs/asset-policy.md`):** Never commit game dumps, `.exe`/`.dll`/`.rpf` files, extracted shaders, captures, or logs. Only `crates/victory-formats/src/bcpack-tables.bin` is exempted in `xtask/src/policy.rs`. Always verify with:
   ```powershell
   cargo xtask policy
   ```
2. **No DRM/auth circumvention:** User-supplied titles must be legally obtained and live outside the repository.
3. **Shader & pipeline caches:** Because `emit::host_caps_key()` is part of the shader cache and pipeline library keys, changing `min_wave` or `barycentrics_supported` automatically creates separate cache entries. When warming offline (`victory translate --cache --pipelines`), ensure the host GPU probe runs (or test on the target Vega GPU) so cached shaders use Vega settings.
4. **Unsafe & lints:** Workspace lints deny `unsafe_op_in_unsafe_fn` and `unused_must_use`, and warn on `undocumented_unsafe_blocks`. Always keep `// SAFETY:` comments accurate and run clippy with `-D warnings`.

## 4. Build, Test, and Verification Commands

```powershell
# Policy & lint checks
cargo xtask policy
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Unit + Vega GPU hardware tests (single-threaded for GPU tests)
cargo test -p victory-shader --lib -- --test-threads=1
cargo test -p victory-gpu --lib --locked
cargo test --workspace --locked

# Build release + diagnostics binaries
cargo build --release -p victory --features victory-gpu/diagnostics --locked
cargo build --profile diag -p victory --features diagnostics,victory-gpu/diagnostics --locked
```

### Recommended Vega iGPU Runtime Environment (`--console series-s`)

```powershell
$env:VICTORY_MESH_EMULATION      = '1'
$env:VICTORY_MESH_BUFFER_MB      = '16'
$env:VICTORY_CARD_BUFFERS_MIN_KB = '4'
$env:VICTORY_CARD_BUFFERS_MB     = '4096'
$env:VICTORY_AMBIENT             = '0.6'
$env:VICTORY_LOG                 = 'info'
```

For `DEVICE_REMOVED` (`0x887A0007` / `DEVICE_HUNG`) or PSO refusal debugging, use the `diag` binary with:
```powershell
$env:VICTORY_D3D12_DEBUG = '1'
$env:VICTORY_D3D12_DRED  = '1'
```

## 5. Known Vega Hardware Caveats

- **Pre-existing GCN shader test quirks:** `fspx_programs_run_with_the_hardwares_dot_products` and `image_atomics_of_64_bits` fail on GCN 5 hardware identically to Polaris (`README.md`).
- **Custom `attributeAtVertex` / weighted barycentric shaders:** Unsupported on hosts without `BarycentricsSupported` and rejected cleanly with an explicit error instead of emitting invalid DXIL.
- **Mesh shaders with cross-lane ops:** Still rejected in `lower.rs::cross_lane` (SM 6.6 `WaveSize` does not apply to mesh stages, and emulated mesh CS does not yet emulate cross-lane ops via LDS).
