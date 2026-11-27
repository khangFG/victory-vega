# VIctory — Vega build

Runs Grand Theft Auto VI (Xbox Series X|S) natively on Windows from files the
user supplies, tuned for **AMD Radeon RX Vega (GCN 5)** GPUs (including low-VRAM
Vega APUs and discrete Vega 56/64). Combines `victory-polaris` with improvements
harvested from `VIctory-RDNA1-Hybrid` (`BarycentricsSupported` fallback,
wave-lane capability probing, `residency.rs` memory pressure/OOM backoff),
**Wave32-on-Wave64 compute lowering** (`wave32_in_64`), GCN 5 raw-structured
buffer lowering (`8404297`), upstream `Marvisak/VIctory` sync (`63bcca4` + PR
`#2389`), and translate/FPS optimizations (`08949a9`). See
[`AGENTS.md`](AGENTS.md), [`progress.md`](progress.md), and [`pull.md`](pull.md).

This fork is **not** identical to upstream `main`: it adapts the runtime and
shader translator for GCN 5 constraints and low-VRAM systems:

| | This fork (`VIctory-Vega`) | Upstream `Marvisak/VIctory` (`63bcca4`) |
|---|---|---|
| Target GPU | **AMD Radeon RX Vega** (GCN 5, Wave64-only, no hardware mesh shaders, no `SV_Barycentrics`, element-counted `V#.NUM_RECORDS`) | AMD RX 6700 XT (RDNA 2, Wave32+64, native mesh shaders, `SV_Barycentrics`) |
| D3D12 | Shader Model **6.6** (`WaveLaneCountMin = 64`), no hardware mesh shaders | SM 6.6 + hardware mesh shaders |
| RAM / VRAM | **16–32 GB RAM** / **512 MiB–2 GiB UMA** (or discrete Vega 56/64) | 32 GB RAM / 12 GB VRAM |
| Profile | **Series S** (`--console series-s`) | Series X |

> **Upstream & Vega summary:** Synced with upstream `Marvisak/VIctory`
> (`main` `63bcca4`), preserving Polaris/Series S features (`mesh_emu`, BCPack,
> offline prebuild, `VICTORY_BIND_CACHE`, `VICTORY_TEXTURE_BACK`,
> `VICTORY_AMBIENT`), Vega extensions (`wave32_in_64`, non-barycentric PS
> fallback, `residency.rs`, low-VRAM mesh buffer sizing, GCN 5 raw-structured
> views), on-demand mip tiles (`#2389`, `VICTORY_LEVELS_ON_DEMAND=1`), and
> translate/FPS performance PRs (`#2662`, `#2664`, `#2665`, `#2958`, `#2051`,
> `#2848`).

## Current Status (RX Vega 7 Verified)

**The 3D world loads and renders on AMD RX Vega 7 (GCN 5)** with only minimal
texture quirks. Root cause of the earlier black 3D world was GCN 5 D3D12
`StructuredBuffer` hardware bounds (`V#.NUM_RECORDS` counts elements instead of
bytes), fixed in `8404297` by unifying 4-byte-aligned structured buffer accesses
and host views onto `R32_TYPELESS` + `FLAG_RAW`. Current work focuses on
translation speed, frame-time optimization, and remaining texture/shader polish.

| Subsystem | Today | Needs |
|-----------|-------|-------|
| Loader | Runs the title's executable and its DLLs in our process; crash reports name the faulting code and are kept in `crashes.log` | – |
| Runtime services | One signed-in offline user, licences, packages, saves that survive a kill at any step, text entry dialog | Speech is silent; the dialog and relaunch are untested live |
| Network | Offline by design: the title cannot reach the network | – |
| Files and streaming | Archives and streamed data read, zlib and BCPack decoded in parallel | Loading is limited by writes into title memory |
| Input | XInput pads, keyboard, keyboard stand-in pad | Trigger rumble |
| Audio | Menu, video and loading sound; XMA voices and Opus streams decode | Reverb is silent; Opus results unverified against the title |
| GPU: menus | Draws, compute and presentation on host D3D12; multi-plane PresentX compositing; only changed title memory is copied | Frame pacing drifts on one screen in release builds |
| GPU: world | **Renders 3D scene on RX Vega 7**: mesh emulation (`mesh_emu`), `wave32_in_64`, non-barycentric PS fallback, raw-structured unification (`8404297`), indirect execution, heap-indexed views, title memory copies, depth twins, placed texture heaps (`placement`), wave-coalesced marks (`#2958`), parallel refresh (`#2051`), pooled copy-back (`#2848`), and on-demand mip tiles (`VICTORY_LEVELS_ON_DEMAND=1`) | Higher FPS (`SHADER_HEAPS` lock, batched guards, row-scoped detile) & remaining texture quirks |
| Shaders | Console GPU code recompiled to DXIL (`translate --cache`), `wave32_in_64` + `EXEC` guard fixes, fast BDD/IR/bitcode emitter (`#2662`/`#2664`/`#2665`) | Remaining skipped/refused shaders & merged geometry stages |
| Textures | Console tiled layouts and detiling + `VICTORY_LEVELS_ON_DEMAND=1` (~1 GiB VRAM saved on Vega APU) | Render-target metadata planes & partial-row detile (`#2985`) |
| Series S mode | Menus, loading and the 3D world run with the title's own upscaler | Higher sustained FPS & `settings.xml` tuning |

Keep the build paths short. From a checkout about 170 characters deep, the
`blake3` dependency's build script fails in `ml64.exe` with
`A1009: line too long`, even with long paths enabled in Windows, because
its object files land under `target`. Set `CARGO_TARGET_DIR` to a short
absolute path (`$env:CARGO_TARGET_DIR = 'C:\vt'`; `victory.exe` is then
in `C:\vt\release` instead of `target\release`), or check out the
repository nearer the drive's root.

**Polaris / Vega low-memory tuning:** set the minimum shadow-copy size low enough that 4 MiB loose windows are subject to the cap:

```powershell
$env:VICTORY_CARD_BUFFERS_MIN_KB = '4'
$env:VICTORY_CARD_BUFFERS_MB     = '4096'
```

Do **not** set `VICTORY_CARD_BUFFERS_MB=512`: even before the leak was
understood it warned `shadow budget is spent size_mb=50 live_mb=677 limit_mb=512`.

The 2 GiB snapshot buffer pool (`shadow.rs`, `POOL_BYTES`) is hardcoded and
still contributes to peak memory; lowering it is the next thing to try if
growth continues.

`VICTORY_D3D12_WARP=1` is **not** a workaround on 16 GB: it dies with hundreds
of `E_OUTOFMEMORY` on 4 MiB `shared_buffer` allocations. It is diagnostic only.

## Getting started on this machine

Requirements: Windows 10/11 x64, AVX2, Rust from `rust-toolchain.toml`, and the
MSVC C++ build tools (Desktop development with C++ workload). The title itself
must be supplied by the user; none of it is in this repository.

```powershell
copy victory.example.toml victory.toml   # set your own paths; this file is gitignored
cargo test --workspace                   # needs no game data
cargo xtask install-hooks                # asset-policy check on every commit
cargo run -p victory -- config
```

Two binaries are built depending on what you are doing:
- **`release`** (`target\release\victory.exe`): fastest offline `translate --cache --pipelines` and highest gameplay FPS (no `CountingAlloc`, `copystat`, or `VICTORY_CAPTURE` overhead). Note that `VICTORY_KEYS` is behind the `diagnostics` feature, so boot manually or load from a save when using `release`.
- **`diag`** (`target\diag\victory.exe`): includes `VICTORY_KEYS`, `fps`/`copystat` logging, `VICTORY_CAPTURE`, and D3D12 debug/DRED diagnostics. Leave `VICTORY_CAPTURE` and `VICTORY_DUMP_TARGETS` unset when benchmarking FPS.

```powershell
cargo build --release -p victory --locked
cargo build --profile diag -p victory --features diagnostics,victory-gpu/diagnostics --locked
```

### 1. Warm the caches

Run this once with `target\release\victory.exe`, and **again every time the
shader emitter changes** (`shader_cache` is keyed by the translator hash):

```powershell
$env:VICTORY_MESH_EMULATION = '1'
$env:VICTORY_LOG = 'warn'
Remove-Item Env:VICTORY_SKIP_PIPELINES -ErrorAction SilentlyContinue

target\release\victory.exe translate --cache --pipelines --jobs 8 `
  <game_dir>\shaders.rpf `
  <game_dir>\game_scarlett_master.exe `
  <game_dir>\XboxBvhBuilder.dll `
  <game_dir>\amd_fidelityfx_upscaler_xbox.dll
```

It takes several minutes and reports how many programs were skipped and why.
`--pipelines` also builds the host pipeline library for your GPU and driver.

### 2. Run

For maximum gameplay FPS (without `VICTORY_KEYS` / capture / profiling):

```powershell
Remove-Item Env:VICTORY_CAPTURE -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_DUMP_TARGETS -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_GPU_TIMING -ErrorAction SilentlyContinue

$env:VICTORY_MESH_EMULATION      = '1'
$env:VICTORY_MESH_BUFFER_MB      = '16'
$env:VICTORY_LEVELS_ON_DEMAND    = '1'
$env:VICTORY_CARD_BUFFERS_MIN_KB = '4'
$env:VICTORY_CARD_BUFFERS_MB     = '4096'
$env:VICTORY_SKIP_PIPELINES      = '39f0e9aeb156a61c'   # CS_ScreenProbes_ActivateRays: ~85 s and 1.3 GB of compile
$env:VICTORY_AMBIENT             = '0.6'
$env:VICTORY_LOG                 = 'warn'

target\release\victory.exe run <game_dir>\game_scarlett_master.exe --console series-s --execute
```

If you need automated `VICTORY_KEYS` to clear first-boot prompts or want
`fps`/`copystat` readouts, use `target\diag\victory.exe` with
`$env:VICTORY_LOG = 'info'` (and `VICTORY_KEYS` if needed), keeping capture and
target dumping disabled unless debugging rendering:

```powershell
$env:VICTORY_KEYS = '10:Y,20:Enter,30:Y,40:Enter,50:Enter,60:Y,70:Enter,80:Enter,100:Y,110:Enter,130:Enter,150:Y,160:Enter,180:Enter,210:Y,220:Enter,240:Enter,270:Enter,300:Enter,330:Enter,360:Enter,390:Enter,420:Enter,450:Enter,480:Enter,510:Enter,540:Enter,570:Enter,600:Enter,630:Enter,660:Enter,690:Enter,720:Enter,750:Enter,780:Enter,810:Enter,840:Enter,870:Enter,900:Enter'
$env:VICTORY_LOG  = 'info'
target\diag\victory.exe run <game_dir>\game_scarlett_master.exe --console series-s --execute
```

Notes learned the hard way:

- Use `--console series-s`. Series X wants ~6.5 GB of physical-page commit and
  hit Windows error 1455 here. **The Windows paging file was not changed**, by
  decision; Series S boots fine with the default.
- The account picker reads `[Y] PLAYER [A]`. Enter maps to pad A, `Y` maps to
  the Y key. Enter-only scripts stall at 14–18 draws/frame.
- Start the shell with a long timeout (e.g. `-Timeout 10800000`). A 2400 s
  timeout once killed a healthy run. The world takes ~20–25 min to load after
  the keys clear first boot.
- Use potato `settings.xml` and a manual save.
- A background shell captures output with `Tee-Object -FilePath <log>` so the
  counts below can be measured after a crash.

### 3. Verify

```powershell
$log = '<your err log>'
'shader does not translate : ' + (Select-String -LiteralPath $log -SimpleMatch 'shader does not translate').Count
'host pipeline refused     : ' + (Select-String -LiteralPath $log -SimpleMatch 'host pipeline refused').Count
'shader translated         : ' + (Select-String -LiteralPath $log -SimpleMatch 'shader translated').Count
'device removed            : ' + (Select-String -LiteralPath $log -SimpleMatch 'device removed').Count
Select-String -LiteralPath $log -SimpleMatch 'host mesh shader support mode=Emulate sm66=true hardware=false'
Get-ChildItem $env:VICTORY_CAPTURE -Filter '*.png' | Sort-Object Length -Descending | Select-Object -First 8 Name,Length
```

A solid-black frame is ~6,121 bytes at 1920×1080; a full-color frame is
1.4–2.5 MB. Because 2D PresentX planes can also produce a large PNG, verify
with `victory_gpu::host` logs showing hundreds/thousands of draws per frame
or inspect the captured frame directly. See [`progress.md`](progress.md) for
current Vega benchmark baselines and recommended `settings.xml` adjustments.

## What was deliberately NOT ported

These features come from other forks but are known to break this lineage or
this machine. Do not add them without a specific reason:

`VICTORY_THIN`, `VICTORY_ASYNC_SUBMIT`, `VICTORY_JOIN_PARTS`, `VICTORY_RELAY`,
`VICTORY_LAND`, `VICTORY_CARD_STEPS`, `VICTORY_SHOW_AFTER`,
`VICTORY_TEXTURE_COPY`, `VICTORY_INDIRECT_REST`, and the `9fea743`
shader-cache format wholesale.

(`VICTORY_LEVELS_ON_DEMAND=1` was originally excluded from the Polaris baseline,
but was safely re-introduced via upstream PR `#2389` atop `63bcca4` and saved
~1,026 MiB of VRAM on Vega 7.)

The texture budget gate was *removed* (`dbe60ff`) — it refused a texture the
world needed even with no override. WARP is diagnostic only (see above).

## Commands

```
victory inspect <exe|dll> [--imports] [--exports]
victory census [paths...] [--missing] [--strict]
victory lib-exports <import.lib> [--list]
victory shader-info [paths...] [--verbose] [--disasm] [--effects]
victory translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
victory translate [paths...] --cache [--pipelines] [--jobs N]
victory run <exe> [--execute] [--unimplemented trap|zero] [--strict]
            [--console series-x|series-s] [-- <title arguments>]
victory config
```

`translate --jobs N` requires a positive worker count. Omit it to use the
default worker count.

Logging: `VICTORY_LOG=victory_loader=trace,info` (a release build logs
info and above).

### Windows shader-test validator

Tests use `dxil.dll` to validate SM 6.6 containers. An older SDK validator may
load but reject them with `Unknown shader model 'ps_6_6'`. Use a current x64
validator from [DXC releases](https://github.com/microsoft/DirectXShaderCompiler/releases)
or a current SDK and set `VICTORY_DXIL` for the test session:

```powershell
$env:VICTORY_DXIL = 'C:\tools\dxc\bin\x64\dxil.dll'
cargo test -p victory-shader --lib --locked -- --test-threads=1
```

The baseline has 3 hardware-dependent test failures
(`permutes_read_the_lane_named_and_zero_from_lanes_left_out`,
`fspx_programs_run_with_the_hardwares_dot_products`,
`image_atomics_of_64_bits`); they fail identically on a clean checkout of the
base commit and are not regressions. Otherwise: 251 passed, 3 ignored.

## Debugging switches

Build them in with `cargo build --profile diag -p victory --features diagnostics`.
Switches below are off when unset or `0`.

| Variable | Effect |
|----------|--------|
| `VICTORY_LOG=<target>=debug,...` | debug and trace output |
| `VICTORY_STACKS=20,60` | every thread's stack at those seconds |
| `VICTORY_WATCH=<rva>,...` | logs a title u32 when it changes |
| `VICTORY_POKE=<rva>:<byte>,...` | writes a title byte twice a second |
| `VICTORY_KEYS=30:Enter,35:X` | presses keys at those seconds, focus or not (needs `diagnostics` feature) |
| `VICTORY_CRASH_BREAKPOINTS=1` | first-chance breakpoints in `crashes.log` |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | saves title memory on a crash |
| `VICTORY_CAPTURE=<dir>` | saves presented frames (powers of two, or `VICTORY_CAPTURE_FRAMES=<list>`) |
| `VICTORY_CAPTURE_WORLD=<list>` | frames at offsets from the first 500-draw frame |
| `VICTORY_DUMP_TARGETS=1` | with the above, saves every render target of those frames |
| `VICTORY_DUMP_WRITERS=<hash>,...` | saves textures a listed shader hash writes |
| `VICTORY_PEEK=<hex address>[:<bytes>],...` | logs title memory and view state in listed frames |
| `VICTORY_D3D12_DEBUG=1` | host debug layer |
| `VICTORY_D3D12_GBV=1` | with the above, GPU-based validation (bad descriptor use reported, not faulted; slow) |
| `VICTORY_D3D12_DRED=1` | on a GPU failure, the allocation hit and the commands in flight |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | skip host pipeline creation and omit draws and dispatches, including indirect ones, when the pipeline contains a listed 16-digit shader hash |
| `VICTORY_GPU_TIMING=1` | with the above, each list of a submission run and timed on its own; slow lists logged with their slowest commands; a device removal names the list it happened in and lists all its commands |
| `VICTORY_TRACE_SHADERS=<hash>,...` | with `VICTORY_D3D12_DRED`, every draw and dispatch of a listed shader logged with what it binds |
| `VICTORY_D3D12_WARP=1` | host device on the software adapter (slow; cannot reset the driver) |
| `VICTORY_PIPELINE_LOG=<file>` | a line per title pipeline created, first used, and made or loaded |
| `VICTORY_PREBUILD_KEYS=<file>` | list pack keys without making anything |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | translated shader loops stop after `n` iterations per lane instead of 2^20 |
| `VICTORY_MESH_EMULATION=1\|0` | force or disable compute+vertex mesh emulation |
| `VICTORY_MESH_BUFFER_MB=<n>` | intermediate buffer in MiB (`1..=1024`) |
| `VICTORY_LEVELS_ON_DEMAND=1` | streamed textures allocate 64 KiB tiles only for mapped mip levels |
| `VICTORY_AMBIENT=<level>` | constant ambient light for unlit surfaces (`off` for none) |
| `VICTORY_CARD_BUFFERS_MB=<n>` | cap on host copies of title buffers |
| `VICTORY_CARD_BUFFERS_MIN_KB=<n>` | copies smaller than this bypass the cap — set low to make the cap effective |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | records what the title plays (our process only) |
| `VICTORY_XMA_DUMP=<dir>` | saves the first 32 XMA voices' context and input |

Every build appends unhandled exceptions, panics and host GPU failures to
`crashes.log` in the title's data folder.

## Repository policy

This repository contains **source only**. No game data, captures, saves,
local configuration or paths to a user's files are committed. `.gitignore` is
the first line of defence; `cargo xtask policy` (also run by the pre-commit
hook in `.githooks/`) is the authoritative check. See `docs/asset-policy.md`.

## Credits

This fork would not exist without the people who did the hard parts first.
Fixes were ported from these forks (commit hashes are where the idea came
from, not necessarily the hash in this tree):

| Fork | What we took |
|------|--------------|
| [yoraudev/VIctory](https://github.com/yoraudev/VIctory) — `gpu/mesh-emulation` @ `d62ba28` | The baseline. Compute + unpack-vertex mesh emulation; the only lineage that drew this title's mesh geometry on Polaris |
| [Marvisak/VIctory](https://github.com/Marvisak/VIctory) — PR #1997, `fdc5ff5` (Yaekai, swarm, Blitz Studios) | Multi-plane PresentX compositing — the fix for the black intro, menus and HUD |
| [FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY) | `11cdbec` bindless image dims / 8-dword size loads; `da327bf` unhostable descriptor dwords dropped instead of refusing the shader; `212a81e` `v_permlanex16_b32` (itself ported from JMAN `39979ac`); `7035d5d` unmatched `v_interp_p1` and `v_cvt_f32_f64`; `946be86` f64 abs/neg; `c156d64` weighted interpolation; `acf478b` the original bind-cache idea |
| [JozefDeTremmerie/CCS03](https://github.com/JozefDeTremmerie/CCS03) | `7d0087c` wave64 mask restored from scalar copies; PR #999 (`f5c03f3`, by cxy0072019-sketch) `VICTORY_TEXTURE_BACK` |
| [naixue2233/Victory-naixue2233](https://github.com/naixue2233/Victory-naixue2233) and [Itsbatt/victory-unified](https://github.com/Itsbatt/victory-unified) | `9896391`, `90228a7`, `a3d3bc9` — descriptor-selection and lane-mask recovery research (the `ir.rs::unspill` variants did not apply to this tree; the applicable parts were already present) |
| [codec-d/cool_fox](https://github.com/codec-d/cool_fox) `ca1d519`, [GabrieelDuart/GabeGTA6](https://github.com/GabrieelDuart/GabeGTA6), [Op-lite/victory-scarlett](https://github.com/Op-lite/victory-scarlett) | Bind cache (`VICTORY_BIND_CACHE`) and low-VRAM profiling work |
| [kzkvf88zhs-boop/ANYDEV](https://github.com/kzkvf88zhs-boop/ANYDEV), [shadany7824/LowVI](https://github.com/shadany7824/LowVI), [techselect/VIctory](https://github.com/techselect/VIctory), [Corgansm/victory-corgi](https://github.com/Corgansm/victory-corgi), [soldsawn/victory-nvidia-8gb](https://github.com/soldsawn/victory-nvidia-8gb) | Descriptor tracking by `s_cselect`/`s_cmov`, reused-register I/O, 8 GB memory policy research |

Also consulted while chasing the same problems:
[vi-7900xtx](https://github.com/gleaf-6/VIctory-7900XTX-),
[victory-8gb-linux](https://github.com/meeyao/victory-8gb-linux.git),
[victory-laptop-8gb](https://github.com/lordon1a/victory-laptop-8gb.git),
[Victory-Rx580](https://github.com/kauazaniboni/Victory-Rx580.git),
[vilocity](https://github.com/voidopcode/vilocity.git),
[DexNom's PSO fixes](https://github.com/naixue2233/Victory-naixue2233),
and the upstream VIctory contributors behind PRs #1240 and #999.

A QA pass on this fork caught a wrong 2-bit-per-lane selector in the
`v_permlanex16` lowering; the fix is in `36174f2`.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
