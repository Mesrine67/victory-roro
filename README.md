# victory — rorodev6's build

A Windows runtime for **an Xbox Series X|S title**, run from game files you own yourself. This branch is maintained by
**[rorodev6](https://github.com/rorodev6)**. Since 2026-11-25 it is based on **shadowdog00's** build, which runs the
title as the smaller console (Xbox Series S) and is about three times faster than the previous `main`. It is tested on an
**AMD Radeon RX 6700 XT (12 GB)**.

The title's original x86-64 code runs unchanged inside the process; the runtime provides the Xbox/GDK platform beneath it
and translates the console's GPU data and shaders (D3D12.X / GFX10 → D3D12 / DXIL).

> No game data is included or distributed here. You need your own copy of the game.

## What this build is

| Part | Source |
|------|--------|
| Base runtime | [Marvisak/VIctory](https://github.com/Marvisak/VIctory) |
| Fork line | [fxceless/victory-gta6](https://github.com/fxceless/victory-gta6) |
| Series S mode, upscaler stand-in, pooled GPU memory, faster card-copy hashing | [shadowdog00/vi-pc-combo](https://github.com/shadowdog00/vi-pc-combo), branch `victory/main` (`692519e`) |
| Shader fixes carried over | [uiliannn/VItoriaBr](https://github.com/uiliannn/VItoriaBr) (see below) |
| FPS counter, shader fixes ported, AMD preset, `play.ps1`, `AGENTS.md` | **rorodev6** (this branch) |

The previous `main` (VItoriaBr + fxceless + low textures, ~3 fps) is kept on the branch `full-build-2026-11-25`.

### rorodev6's changes on top of shadowdog's build

- **FPS counter** in the top-left corner of the game window (`crates/victory-gpu/src/fps_overlay.rs`). It is a separate
  transparent window, so it costs no frame time. `VICTORY_FPS_OVERLAY=0` turns it off.
- **Shader translator fixes ported from VItoriaBr**, for programs the game uses that did not translate before:
  - `v_cvt_f32_f64` (water volumetrics compute shaders), VItoriaBr `3052431`;
  - `v_permlane16_b32` / `v_permlanex16_b32` (water transmittance), from VItoriaBr `31714de`;
  - buffer-view addresses chosen by a computed heap index (`SLOT_VIEWS`), VItoriaBr `b62bbc0`.
- `play.ps1` with shadowdog's preset, Series S by default.

## Status (2026-11-25, RX 6700 XT 12 GB, Ryzen, 32 GB RAM)

| Stage | State |
|-------|-------|
| Boot, menus, intro video | ✅ Works |
| Opening cutscene, prologue | 🟡 **7-9 fps** (113-141 ms a frame), up from ~3 fps |
| Characters | ✅ Bodies, faces and hair drawn |
| Textures | 🟡 Some walls, posters and wire mesh differ from launch to launch |
| Ray tracing | ⚪ Off (its structure builder is not emulated) |

Where a frame goes on the RX 6700 XT: about 70-80 ms preparing it (copying the title's memory to the card), 60-75 ms
copying results back (partly overlapping), and only 22-40 ms of GPU work. The single submitting thread is the limit,
not the GPU.

## Easiest way: ask an AI assistant

Open this folder in an AI coding assistant (Claude Code, Cursor, Copilot, …) and say *"set this up and launch the
game"*, or *"look at my err.txt"* after a crash. `AGENTS.md` tells it how to build, run and read the logs.

## Launching and logs

```powershell
cargo build --locked --profile diag -p victory --features diagnostics
powershell -NoProfile -ExecutionPolicy Bypass -File .\play.ps1 -Game "D:\Games\GTA VI\game_scarlett_master.exe"
```

The path is remembered after the first run. `-SeriesX` runs as the larger console (about half the frame rate). Each run
writes `logs\<date_time>\err.txt` (the runtime's log), `out.txt` and `run.txt`.

## Windows 10

The runtime uses the system's Direct3D 12. Windows 10's does not provide Shader Model 6.6, which the game needs, and
`victory.exe` does not load the D3D12 Agility SDK yet, so Windows 11 is required for now.

## License

GPL-3.0, like the projects it is built from. The original project's README is in `README.upstream.md`.
