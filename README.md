# VIctory — Vega build

Runs Grand Theft Auto VI (Xbox Series X|S) natively on Windows from files the
user supplies, tuned for **AMD Radeon RX Vega (GCN 5)** GPUs (including low-VRAM
Vega APUs and discrete Vega 56/64). Combines `victory-polaris` with improvements
harvested from `VIctory-RDNA1-Hybrid` (`BarycentricsSupported` fallback,
wave-lane capability probing, `residency.rs` memory pressure/OOM backoff) plus
**Wave32-on-Wave64 compute lowering** (`wave32_in_64`) and low-VRAM mesh buffer
sizing. See [AGENTS.md](AGENTS.md) for architecture and workflow details.

This fork is **not** the upstream feature-complete tree. It started from the
`victory-polaris` RX 560 XT lineage and adds Vega-specific fixes:

| | Test machine (the one this README describes) | Upstream's tested machine |
|---|---|---|
| GPU | **AMD RX 560 XT 8 GB** (Polaris 10 / GCN 4, device `0x67DF`) | AMD RX 6700 XT (RDNA 2) |
| D3D12 | Feature Level **12_0** — no hardware mesh shaders, no SM 6.6 | SM 6.6 |
| RAM | **16 GB** (pagefile left at Windows defaults, by choice) | more |
| Profile | **Series S** (`--console series-s`) | Series X |

Everything below comes from actually running the game on that machine. It is
not a claim about other hardware.

## Status (2026-11-27)

**Not playable. The 3D world is still black.** The UI, menus, map, audio and
the 2D presentation planes work, and the host submits tens of thousands of
draws, but the rendered 3D scene has never appeared on screen on this machine.

| Stage | Observed on this machine |
|---|---|
| Boot, first-boot screens, profile/save | works (needs scripted keys, see below) |
| Intro cinematic audio, sound, music, ambience | works |
| Money UI, phone message UI, map (CENTER / 2D VIEW / ADD MARKER / SET DESTINATION / EXIT) | works |
| 2D presentation planes (splash, loading art, UI) | full color (were solid black before the compositor fix) |
| **3D world / first mission's cinematic** | **black** — see the warning below |
| World geometry submitted | confirmed: up to **17,285 draws/frame**, emulated mesh pipelines active |
| Live gameplay frames | reached frame ~46,000 with ~5,900 draws/frame; black-frame warnings, very slow (~1 fps or worse) |
| Memory | grows without bound in long sessions (see [Memory](#memory-the-last-open-problem)); a fix is committed but has not yet been verified in a full run |

> **Do not mistake the captures for the world.** `frame-17000.png`,
> `frame-18000.png`, `frame-20000.png` and `frame-32768.png` are 1.4–2.5 MB
> full-color images, but they show the **2D PresentX presentation plane** —
> splash and loading art — composited by the fix below. A solid-black frame is
> ~6,121 bytes at 1920×1080, so a large PNG proves that *something* was
> presented; it does **not** prove that 3D geometry or the world shaders are
> working. The user's own check confirms the 3D scene remains black.

Translation counts from the diagnostic run:

- `shader does not translate`: **14** (hundreds/thousands before the shader fixes)
- `shader translated`: **2,511**
- These errors are real and were reduced, but clearing them did **not** by
  itself restore the 3D world.

## Why this fork exists

Getting from a completely black window to a working UI with sound, and getting
the host to submit real world geometry, took several separate fixes. Each was
found from the log, not guessed. **None of them has made the 3D world visible
yet.**

**1. The GPU has no mesh shaders.** The title's world is entirely mesh-shader
geometry. On this card the host reports:

```
host mesh shader support mode=Emulate sm66=true hardware=false
```

That comes from the upstream `gpu/mesh-emulation` lineage (`d62ba28`), which
is the base of this fork. Without it: `no host pipeline: the host GPU has no
mesh shaders (or no shader model 6.6)` and `host pipeline refused 0x80070057`.

**2. The black intro/UI was a compositing bug, not a render bug.** Series S
presents **two** PresentX planes: plane 0 is the 1920×1080 world, plane 1 is a
3840×2160 layer carrying the Rockstar splash, intro video, menus, loading art
and HUD. The host only presented plane 0, so everything the player was
supposed to see never reached the swap chain — hence "audio and clicks work
but the screen is black". Ported from upstream PR #1997 (commit `376cd51`,
merged as `ea7625e`); `queue.rs` walks all planes at a 0x48-byte stride and
`host.rs` alpha-composites them. After this, the loading art appeared.

**3. World shaders were being dropped — but fixing that was not enough.** The
translator rejected GFX10 constructs the title's world shaders use:

```
a descriptor read only as data
a descriptor neither loaded nor built
a descriptor chosen by control flow
barycentrics read directly
a lane mask read as data
a register joining a lane mask and data, read as data
a lane-mask join of an untracked mask
v_cvt_f32_f64
v_permlanex16_b32
```

Those pixel/compute programs were refused, so their draws emitted no pixels.
The UI survived because its shaders are simple. Fixes were ported from the
ZJOU / naixue / unified / CCS03 forks and merged in `74dacbe`; a QA review
caught a wrong 2-bit-per-lane selector in the `v_permlanex16` lowering, fixed
in `36174f2` to the ISA's 4 bits per lane. The refusal count fell dramatically
but the world is still black, so another cause remains.

**4. Two OOM classes had to be fixed separately.**

- `memory::table()` used a `Vec` that doubled on a 1 MiB realloc during page
  table rewrites. Exact-size allocation fixed it (`9dd381b`).
- Loose host copies of title memory (4 MiB windows, used for the title's own
  interface-vertex allocations) were **explicitly never freed**. The source
  said so. With `VICTORY_CARD_BUFFERS_MIN_KB=16384` every such copy bypassed
  the nominal shadow cap, so private bytes climbed past 23 GB on a 16 GB
  machine and allocations started failing. `65ee9f9` evicts copies no view
  still references when the budget is reached, returning both the RAM and the
  budget via `Shadow::drop`.

Other ports that matter on 8 GB / 16 GB: `VICTORY_BIND_CACHE`, an 8 GB memory
policy, `VICTORY_TEXTURE_BACK`, removal of the texture budget gate (it refused
a texture the world needed, leaving holes), keeping the title's own FidelityFX
upscaler instead of forcing FSR4, nulling outputs of unimplemented pipelines
(instead of crashing), and `VICTORY_AMBIENT` as a stand-in for missing bounce
light.

## Memory: the last open problem

The eviction fix in `65ee9f9` is compiled and committed but **has not yet been
verified in a completed run**. To exercise it, set the minimum shadow-copy
size low enough that the 4 MiB loose windows are actually subject to the cap:

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

Two binaries are needed. **The plain release binary ignores `VICTORY_KEYS`** —
scripted keys are behind the `victory` crate's `diagnostics` feature
(`gameinput.rs:1253`). Use `diag` for any run that must get past first boot:

```powershell
cargo build --release -p victory --features victory-gpu/diagnostics --locked
cargo build --profile diag -p victory --features diagnostics,victory-gpu/diagnostics --locked
```

### 1. Warm the caches

Run this once, and **again every time the shader emitter changes** — the cache
is keyed by the translator, so a new emitter makes every cached entry stale.

```powershell
$env:VICTORY_MESH_EMULATION = '1'
$env:VICTORY_LOG = 'warn'
Remove-Item Env:VICTORY_SKIP_PIPELINES -ErrorAction SilentlyContinue
target\release\victory.exe translate --cache --pipelines --jobs 4 `
  E:\gtaxbox\shaders.rpf `
  E:\gtaxbox\game_scarlett_master.exe `
  E:\gtaxbox\XboxBvhBuilder.dll `
  E:\gtaxbox\amd_fidelityfx_upscaler_xbox.dll
```

It takes several minutes and reports how many programs were skipped and why.
`--pipelines` also builds the host pipeline library; it belongs to the GPU and
driver it was made on.

### 2. Run

```powershell
$env:VICTORY_KEYS           = '10:Y,20:Enter,30:Y,40:Enter,50:Enter,60:Y,70:Enter,80:Enter,100:Y,110:Enter,130:Enter,150:Y,160:Enter,180:Enter,210:Y,220:Enter,240:Enter,270:Enter,300:Enter,330:Enter,360:Enter,390:Enter,420:Enter,450:Enter,480:Enter,510:Enter,540:Enter,570:Enter,600:Enter,630:Enter,660:Enter,690:Enter,720:Enter,750:Enter,780:Enter,810:Enter,840:Enter,870:Enter,900:Enter'
$env:VICTORY_MESH_EMULATION = '1'
$env:VICTORY_MESH_BUFFER_MB = '16'
$env:VICTORY_SKIP_PIPELINES = '39f0e9aeb156a61c'   # CS_ScreenProbes_ActivateRays: ~85 s and 1.3 GB of compile
$env:VICTORY_CARD_BUFFERS_MIN_KB = '4'
$env:VICTORY_CARD_BUFFERS_MB     = '4096'
$env:VICTORY_AMBIENT        = '0.6'
$env:VICTORY_CAPTURE        = 'E:\gtaxbox-tools\capture-folder'   # use a FRESH folder
$env:VICTORY_CAPTURE_FRAMES = '1,16,128,1024,4096,8192,16000,16384,17000,18000,20000,24000,27337,32768,40000,45968'
$env:VICTORY_LOG            = 'info'

target\diag\victory.exe run E:\gtaxbox\game_scarlett_master.exe --console series-s --execute
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
1.4–2.5 MB. But **a large PNG only proves that the 2D presentation plane was
composited — it is not proof that the 3D world renders.** Check what is in the
image before claiming progress. Draws in world frames should be in the hundreds
to thousands; a few draws means the title is still stuck on a first-boot
screen. The 3D scene being black with high draws is the current, unsolved
failure.

## What was deliberately NOT ported

These features come from other forks but are known to break this lineage or
this machine. Do not add them without a specific reason:

`VICTORY_THIN`, `VICTORY_ASYNC_SUBMIT`, `VICTORY_JOIN_PARTS`, `VICTORY_RELAY`,
`VICTORY_LAND`, `VICTORY_CARD_STEPS`, `VICTORY_SHOW_AFTER`,
`VICTORY_TEXTURE_COPY`, `VICTORY_LEVELS_ON_DEMAND`, `VICTORY_INDIRECT_REST`,
and the `9fea743` shader-cache format wholesale.

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

Logging: `VICTORY_LOG=victory_loader=trace,info` (release logs info and above).

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
| `VICTORY_D3D12_GBV=1` | GPU-based validation (slow) |
| `VICTORY_D3D12_DRED=1` | allocation and commands in flight on a GPU failure |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | omit draws/dispatches containing a 16-digit shader hash |
| `VICTORY_GPU_TIMING=1` | time each list separately; slow lists logged with slowest commands |
| `VICTORY_TRACE_SHADERS=<hash>,...` | with DRED, log every use of a shader with its bindings |
| `VICTORY_D3D12_WARP=1` | software adapter (slow; diagnostic only here) |
| `VICTORY_PIPELINE_LOG=<file>` | a line per title pipeline created/first used/made or loaded |
| `VICTORY_PREBUILD_KEYS=<file>` | list pack keys without making anything |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | cap translated loop iterations per lane (own cache entries) |
| `VICTORY_MESH_EMULATION=1\|0` | force or disable compute+vertex mesh emulation |
| `VICTORY_MESH_BUFFER_MB=<n>` | intermediate buffer in MiB (`1..=1024`) |
| `VICTORY_AMBIENT=<level>` | constant ambient light for unlit surfaces (`off` for none) |
| `VICTORY_CARD_BUFFERS_MB=<n>` | cap on host copies of title buffers |
| `VICTORY_CARD_BUFFERS_MIN_KB=<n>` | copies smaller than this bypass the cap — set low to make the cap effective |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | record what the title plays |
| `VICTORY_XMA_DUMP=<dir>` | save the first 32 XMA voices' context and input |

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
