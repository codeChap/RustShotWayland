# Project understanding

> Scope: whole repo
> As-of: 2026-08-25 · commit `a1022f6`

## What it is

**RustShot Wayland** (`rustshot-wayland`) is a Flameshot-style screenshot tool for **Omarchy / Hyprland**. A long-lived daemon owns the session bus and an optional status-tray icon; short-lived CLI processes (or `dbus-send`) ask it to capture. Interactive capture freezes one monitor’s pixels, then shows a **wlr-layer-shell Overlay** so the user can crop, annotate, and save/copy PNG. It is the Wayland sibling of X11 `rustshot` (referenced from `README.md`); the two can run side by side because this binary claims `org.rustshot.Wayland`, not the X11 name.

## How it runs

- Runtime: single Rust binary, edition 2021. No subcommand → daemon. Any subcommand → D-Bus client (`src/main.rs:16-22`). Requires `WAYLAND_DISPLAY` (`src/capture/grim.rs:18-22`).
- Entry points:
  - `rustshot-wayland` — daemon (`src/daemon.rs:9`)
  - `rustshot-wayland gui|full|screen` — client (`src/cli.rs:16-28`, `src/client.rs:6`)
  - Session D-Bus `org.rustshot.Wayland` at `/` (`src/dbus/mod.rs:11-12`)
  - SNI tray click / “Capture region” (`src/tray/sni.rs:41-63`)
  - Sample user unit `data/systemd/rustshot.service` (binary name in the unit does **not** match this crate; see Open questions)
- Config / env: `~/.config/rustshot-wayland/config.toml` via `dirs::config_dir` (`src/config.rs:86-91`). Missing/invalid file → defaults (`src/config.rs:61-82`). Tracing from `RUST_LOG` / `EnvFilter`, default `info` (`src/main.rs:25-28`).
- Data stores: none. PNGs go to `defaults.save_dir` + strftime `filename_pattern` (`src/config.rs:93-98`) and optionally `clipboard.latest_path` (`src/config.rs:40-57`).
- Background work: daemon thread `rustshot-dbus` (current-thread Tokio + zbus + ksni) (`src/daemon.rs:20-47`). Overlay capture from the tray is `rustshot-tray-capture` (`src/tray/mod.rs:31-47`). `wl-copy` is spawned and waited on a detached thread (`src/export/clipboard.rs:66-68`). No queue, no DB, no cron in-repo.

## Architecture map

One crate, modules as layers. The daemon process splits **D-Bus/tray (Tokio thread)** from **overlay UI (main thread)** so Wayland compositing is not on the bus runtime.

```
CLI / dbus-send / SNI
        │  session bus org.rustshot.Wayland
        ▼
src/dbus/mod.rs  ──BusyGuard──► src/ui (channel UiRequest)
        │                              │
        │  capture pixels              ▼
        ▼                       src/ui/overlay/mod.rs  (blocking event loop)
src/capture/grim.rs             ├ state.rs   data
  libwayshot → grim PPM         ├ draft.rs   in-progress strokes
  hyprctl layout/cursor         ├ selection.rs  frame handles
                                ├ tool_buttons.rs  strip
                                ├ paint.rs   composite
                                └ wl_win.rs  layer-shell + SHM
        │
        ▼
src/canvas  (ToolKind, Annotation, geometry, tiny-skia raster)
        │
        ▼
src/export  file PNG  ·  wl-copy image/png  ·  latest_path sidecar
```

| Dir / file | Role |
|------------|------|
| `src/main.rs` | Parse CLI; daemon vs client |
| `src/cli.rs` | clap: `gui` / `full` / `screen` + `-p -c -d --no-save` |
| `src/client.rs` | Session-bus proxy; maps CLI → `*Flags` methods |
| `src/daemon.rs` | Load config, `WaylandCapture`, spawn D-Bus thread, UI recv loop |
| `src/dbus/mod.rs` | Flameshot-compat D-Bus surface + `submit_overlay` |
| `src/config.rs` | TOML model, `~` expand, auto-save path |
| `src/capture/` | `Screen`, BGRA swizzle, `WaylandCapture` (libwayshot + grim + hyprctl) |
| `src/ui/` | `UiRequest` / `BusyGuard` / overlay |
| `src/canvas/` | Tools, annotations, geometry, widget mocks, raster |
| `src/export/` | PNG file + `wl-copy` |
| `src/tray/` | SNI only (`ksni`); comments mention unused XEmbed |
| `assets/font.ttf` | Embedded via `include_bytes!` (`src/canvas/render.rs:8`) |
| `data/sample-config.toml` | Human sample (path header is stale vs loader) |

Seams that matter:

- **Bus vs UI:** D-Bus methods `spawn_blocking` capture, then `ui_tx.send(ShowOverlay)` (`src/dbus/mod.rs:178-228`). The overlay loop is `ui::run_event_loop` on the main thread (`src/ui/mod.rs:48-64`).
- **One overlay:** `BusyGuard` CAS on `gui_busy` (`src/ui/mod.rs:32-45`). Repeat PrtSc / tray click while the overlay is up is dropped (`src/dbus/mod.rs:42-45`).
- **Capture vs overlay pixels:** capture is RGBA; SHM is Argb8888; `swizzle_rb_opaque` swaps R↔B and forces alpha `0xFF` (`src/capture/mod.rs:16-33`, `src/ui/overlay/wl_win.rs:295-301`).

## Critical paths

### 1. Interactive region capture (write path)

Trigger: `rustshot-wayland gui -c` (or Hyprland `Print` bind, or `graphicCaptureFlags`).

1. Client connects to the session bus and calls `graphicCaptureFlags(path, delay, clipboard, no_save, "")` (`src/client.rs:18-31`).
2. Service acquires `BusyGuard`; if already busy, returns `Ok(())` with no work (`src/dbus/mod.rs:42-45`).
3. Empty path + not `no_save` → `auto_save_path(save_dir, filename_pattern)` (`src/dbus/mod.rs:146-157`).
4. Optional `delay` sleep (`src/dbus/mod.rs:119-122`).
5. `submit_overlay`: `cursor_screen()` then `capture_screen_with_cursor` (`src/dbus/mod.rs:214-216`).
6. `UiRequest::ShowOverlay` on the UI channel; D-Bus **awaits** the oneshot `UiResult` so the CLI process blocks until the overlay closes (`src/dbus/mod.rs:183-194`, `src/client.rs:20-31`).
7. Overlay: `WlWin::new` layer-shell Overlay + exclusive keyboard (`src/ui/overlay/wl_win.rs:111-120`); first paint then grab (`src/ui/overlay/mod.rs:70-92`).
8. User drags a region (`w,h ≥ 4`) → `Mode::Annotating` (`src/ui/overlay/mod.rs:444-450`). Tools / Enter / Ctrl+C.
9. `OverlayState::act` composes (base + vector overlays, crop to selection) then `save_png` and/or `clipboard::copy` (`src/ui/overlay/state.rs:205-235`).
10. Terminal: `UiResult::Done` or `Cancelled` (Esc / window `closed` synthesized as Esc) (`src/ui/overlay/mod.rs:497-499`, `src/ui/overlay/wl_win.rs:222-227`). `BusyGuard` drops when the `ShowOverlay` match arm ends (`src/ui/mod.rs:51-61`).

Silent / soft failure: save or clipboard errors are logged; `act` still returns `Done` (`src/ui/overlay/state.rs:208-221`). `latest_path` write failure is warn-only (`src/export/clipboard.rs:21-28`). Overlay window create/map/blit/grab failure sends `Cancelled` and the D-Bus method still `Ok(())` (`src/ui/overlay/mod.rs:40-45`, `src/dbus/mod.rs:190-194`).

### 2. Full-desktop capture, no UI (write path)

Trigger: `rustshot-wayland full -c` → `fullScreenFlags`.

1. `resolve_save_path` as above (`src/dbus/mod.rs:64-82`).
2. After delay, `CaptureKind::All` → `capture.capture_all()` (`src/dbus/mod.rs:256-261`).
3. `capture_all()` **always** passes `include_cursor = false` even if config says otherwise (`src/capture/grim.rs:93-94`).
4. Non-empty path → `export::file::save_png`; `-c` → `wl-copy` + optional latest copy (`src/dbus/mod.rs:270-281`).
5. Terminal: method returns. If `--no-save` and no `-c`, logs a warning and writes nothing (`src/dbus/mod.rs:282-284`).

Silent: `capture_all` does not use `BusyGuard` — it can run while an overlay is open. libwayshot `screenshot_all` failure falls through to grim (`src/capture/grim.rs:112-118`).

### 3. Overlay annotate (read + mutate, in-memory)

Trigger: overlay already showing a captured `RgbaImage`.

1. Start in `SelectingRegion`; dim whole capture, copy selection rect from `base` (`src/ui/overlay/paint.rs:18-30`).
2. After a valid region, strip of `ToolKind::ALL` (17) + Save + Copy (`src/ui/overlay/tool_buttons.rs:1-22`, `src/canvas/mod.rs:26-44`).
3. Keys `1`–`8` arm only the first eight tools (`src/ui/overlay/mod.rs:514-518`). Remaining tools (stamps `! ? *`, six widgets) are strip-only.
4. Drag tools become `Draft`, then `finalize()` on release (min size / min length) (`src/ui/overlay/draft.rs:86-137`). Highlighter is stored as `Annotation::Pencil` with fixed yellow α=110 / 20px (`src/ui/overlay/draft.rs:7-12,44`).
5. Pixelates are baked into `base` / `dim_base`, not vector-drawn (`src/ui/overlay/state.rs:113-169`, `src/canvas/render.rs:17-20`).
6. Motion bursts collapse to one dirty-rect blit (`src/ui/overlay/mod.rs:146-167`, `src/ui/overlay/paint.rs:70-133`).
7. Enter → `act(false)` (save; copy only if `-c` / `clipboard_pref`); Ctrl+C or Copy button → `act(true)` (force copy) (`src/ui/overlay/mod.rs:485-507`).

Silent: `WlWin::new` takes `screen_origin` but **discards it** and creates the layer surface with output `None` (`src/ui/overlay/wl_win.rs:111-117,147`). Capture is the cursor’s monitor (`src/dbus/mod.rs:214-222`); Hyprland may map the overlay on a different output.

### 4. Daemon start, tray, and ops failure

Trigger: `rustshot-wayland` with no args (Hyprland `exec-once`).

1. `Config::load_or_default` (`src/daemon.rs:10`).
2. `WaylandCapture::new` — fail hard without `WAYLAND_DISPLAY` (`src/capture/grim.rs:18-22`).
3. Spawn `rustshot-dbus`: claim name, serve `Service`, try SNI (`src/daemon.rs:53-97`).
4. `NameTaken` → print `pkill -x rustshot-wayland` and `dbus_main` returns `Ok` (`src/daemon.rs:74-80`). The D-Bus thread then **exits**; the main thread **keeps** `run_event_loop` (`src/daemon.rs:42-50`). Second instance is a UI-loop zombie with no bus and no tray.
5. SNI failure: log and continue with **no tray**. `tray_fallback` is unused (`src/daemon.rs:65,92-96`). Module comment claiming XEmbed fallback is false (`src/tray/mod.rs:1-8`).
6. Tray activate always auto-save path + **clipboard true**, fire-and-forget (drops the oneshot) (`src/tray/mod.rs:34-43`).
7. Tray **Quit** is `std::process::exit(0)` (`src/tray/sni.rs:68-70`). SIGINT is awaited only on the D-Bus thread (`src/daemon.rs:99-101`); `ui_tx` still lives on the main stack so Ctrl-C does not close the UI channel.

Silent: no D-Bus activation; `gui` without a daemon fails at `Connection::session` / method call (`src/client.rs:14-15`). systemd `SIGTERM` is not handled.

## Domain language

| Term | Meaning in this codebase |
|------|--------------------------|
| Daemon | `rustshot-wayland` with no subcommand; owns the bus + UI loop |
| Client | Subcommand process that only calls D-Bus then exits |
| graphicCapture / fullScreen / captureScreen | D-Bus methods; `*Flags` add clipboard + no_save (`src/dbus/mod.rs:23-116`) |
| Overlay | Fullscreen wlr-layer-shell UI over a captured output |
| Selection / frame | Crop rectangle (yellow chrome), not an `Annotation` |
| Handle | N/S/E/W + corners for resize (`src/ui/overlay/selection.rs:7`) |
| Mode | `SelectingRegion` vs `Annotating` |
| Canvas | Committed `Annotation`s + redo + optional armed tool |
| Draft | In-progress drag; becomes `Annotation` on `finalize` |
| Armed tool | `Option<ToolKind>`; `None` means inside-drag **moves** the frame |
| Strip / Hit | Floating toolbar: tools + Save + Copy |
| Stamp | Bare `!` `?` `*` glyph, no bubble |
| Counter | Numbered bubble; live `counter` increments with undo/redo |
| Widget | Drag-to-size UI mock: Button, Input, ImageX, Checkbox, Toggle, Measure |
| Highlighter | Draft-only style; committed as `Annotation::Pencil` |
| original / base / dim_base | Immutable capture; pixelates baked; RGB-halved dim layer |
| Chrome | Selection, handles, strip, hint (dirty-rect AABB) |
| Screen | One output: `x,y,width,height,name` (`eDP-1`, …) |
| latest_path | Extra PNG written on clipboard copy |
| BusyGuard | One overlay at a time |
| CaptureKind | `CursorScreen` / `All` / `Screen(usize)` |

## Invariants and gotchas

- **Stroke style is not configurable.** Default red `[255,50,50,255]` / 4px (`src/canvas/mod.rs:72-77`); tests pin it (`src/canvas/mod.rs:366-369`). Sample config says this is deliberate (`data/sample-config.toml:4-5`).
- **Selection must be ≥ 4×4 px** or it is discarded (`src/ui/overlay/mod.rs:446-449`). Draft line/arrow need `dist2 >= 4`; rect-like drafts need `w,h ≥ 2` (`src/ui/overlay/draft.rs:96-207`).
- **Pixelate block ≥ 2**, counter radius ≥ 4 at overlay construction (`src/ui/overlay/state.rs:94-95`); `pixelate_crop` also `block.max(2)` (`src/canvas/render.rs:510`).
- **Undo of `Annotation::Counter` saturating-decrements** the live counter; redo increments (`src/canvas/mod.rs:165-180`).
- **`push` clears redo** (`src/canvas/mod.rs:160-163`).
- **Config load is fail-open:** bad TOML / unreadable file → defaults (`src/config.rs:61-82`).
- **`full` ignores `include_cursor`.** Single-output paths honor it (`src/dbus/mod.rs:213-216` vs `src/capture/grim.rs:93-94`).
- **Capture prefers libwayshot**, grim is fallback (`src/capture/grim.rs:1,24-31,102-105`). README still lists grim as the grabber.
- **hyprctl cursorpos failure → `(0,0)`** (`src/capture/grim.rs:55-57`), which can pick the wrong monitor.
- **`screen_from_output` uses `logical_position()` + `physical_size()`** (`src/capture/grim.rs:140-149`). On scaled outputs those axes may not match; unverified against libwayshot (Open questions).
- **Layer surface is not bound to the captured output** (`src/ui/overlay/wl_win.rs:111-117,147`).
- **Double-buffered SHM + dirty-rect swizzle** updates only the current buffer’s dirty region (`src/ui/overlay/wl_win.rs:274-331`). The inactive buffer is not copied forward — possible stale tiles on partial blits.
- **Cursor-icon map is incomplete / wrong for some handles:** `Handle::S` glyph `16` maps to `EwResize`; `Handle::E` glyph `96` falls through to `Default` (`src/ui/overlay/selection.rs:95-105`, `src/ui/overlay/wl_win.rs:240-252`).
- **Export errors do not fail the overlay** (`src/ui/overlay/state.rs:208-221`).
- **`wl-copy` is not joined** before `copy` returns (`src/export/clipboard.rs:66-68`) — paste can race.
- **No D-Bus autostart.** Client without daemon does not spawn one.
- **Release `panic = "abort"`** (`Cargo.toml:38`).
- **Unsafe uninit pixel alloc** — callers must fill every byte (`src/capture/mod.rs:5-14`).
- **Tests pin pure math only** (64 unit tests in 8 files). No tests for daemon, D-Bus, grim, overlay loop, export, config, CLI. No CI (no `.github/`, Makefile, justfile).
- **Stale X11 wording** is leftover from the sibling: `submit_overlay` “blocking X11 capture” (`src/dbus/mod.rs:201`); paint “blit to the X11 window” (`src/ui/overlay/paint.rs:3`); sample-config XFixes / X11 selection (`data/sample-config.toml:14-26`); tray XEmbed (`src/tray/mod.rs:1-4`). Code paths are Wayland (`libwayshot` / `grim` / `wl-copy` / layer-shell).

## Where to change what

| If you need to… | Start here |
|-----------------|------------|
| Add a CLI flag or capture mode | `src/cli.rs` then `src/client.rs` + `src/dbus/mod.rs` |
| Change D-Bus names / Flameshot-compat methods | `src/dbus/mod.rs` (`SERVICE_NAME`, `#[interface]`) |
| Change what “gui” captures | `submit_overlay` + `WaylandCapture::cursor_screen` (`src/dbus/mod.rs:204`, `src/capture/grim.rs:59`) |
| Swap capture backend | `src/capture/grim.rs` (`wayshot_one` / `grim_output`) |
| Add / reorder annotation tools | `ToolKind::ALL` (`src/canvas/mod.rs:26`), strip (`src/ui/overlay/tool_buttons.rs`), keys (`src/ui/overlay/mod.rs:514`) |
| Change default red stroke | `Style::default` (`src/canvas/mod.rs:72`) |
| Change overlay compositing / dim / dirty rects | `src/ui/overlay/paint.rs` + `state.rs` |
| Change layer-shell / SHM / input | `src/ui/overlay/wl_win.rs` |
| Change save path / latest sidecar | `src/config.rs`, `src/export/{file,clipboard}.rs` |
| Change tray menu / click | `src/tray/sni.rs`, `spawn_capture` in `src/tray/mod.rs` |
| Config keys / defaults | `src/config.rs` + `data/sample-config.toml` (keep header path in sync with `config_path()`) |

## Hotspots

Single commit (`a1022f6`); no path churn. Load-bearing by role and size:

- `src/ui/overlay/mod.rs` — overlay event loop, tools, keys, drag
- `src/ui/overlay/wl_win.rs` — compositor contract
- `src/canvas/render.rs` — raster + pixelate + font
- `src/dbus/mod.rs` — public IPC + capture dispatch
- `src/capture/grim.rs` — pixels and monitor targeting
- `src/ui/overlay/state.rs` — compose/export and pixelate baking
- `src/ui/overlay/tool_buttons.rs` — 17-tool strip layout

## Open questions

- **Sample config path** says copy to `~/.config/rustshot/config.toml` (`data/sample-config.toml:1`); loader uses `rustshot-wayland` (`src/config.rs:86-91`, `README.md:20`). Code wins.
- **systemd unit** `ExecStart=%h/.cargo/bin/rustshot` (`data/systemd/rustshot.service:6`) vs binary `rustshot-wayland`. README uses Hyprland `exec-once`, not this unit.
- **README “Needs grim”** vs libwayshot-first. Grim is required only if libwayshot fails.
- **hyprctl “optional but expected”** (`README.md:11`) vs required for fallback monitor list and multi-monitor focus (`src/capture/grim.rs:52-53,74-76,185`).
- **Does `logical_position` + `physical_size` work on scaled Hyprland outputs?** Not verified in-repo.
- **Does Hyprland place an unbound Overlay on the captured monitor?** `screen_origin` is unused.
- **SIGINT / SIGTERM vs UI thread:** D-Bus thread waits for Ctrl-C; UI loop may never return. How operators are expected to stop the daemon besides tray Quit / `pkill` is undocumented.
- **Is `id` on D-Bus methods ever used?** Always `""` from the client; discarded as `_id`.
- **Verification gate:** nothing in-repo says to run `cargo test`. Overlay/D-Bus/capture have no mocks.

## Key files

| File | Role |
|------|------|
| `src/main.rs` | Binary entry: tracing + daemon vs client |
| `src/cli.rs` | Public clap surface |
| `src/client.rs` | Session-bus client |
| `src/daemon.rs` | Process model: config, capture, D-Bus thread, UI loop |
| `src/dbus/mod.rs` | `org.rustshot.Wayland` methods + `submit_overlay` |
| `src/config.rs` | TOML config, `config_path`, auto-save |
| `src/capture/grim.rs` | libwayshot / grim / hyprctl capture |
| `src/capture/mod.rs` | `Screen`, Argb8888 swizzle |
| `src/ui/mod.rs` | `UiRequest`, `BusyGuard`, event loop |
| `src/ui/overlay/mod.rs` | Overlay interaction loop |
| `src/ui/overlay/state.rs` | Overlay data + `act`/`compose` |
| `src/ui/overlay/wl_win.rs` | wlr-layer-shell window |
| `src/ui/overlay/draft.rs` | In-progress annotations |
| `src/ui/overlay/selection.rs` | Frame handles |
| `src/ui/overlay/tool_buttons.rs` | Tool strip |
| `src/ui/overlay/paint.rs` | Composite + dirty rects |
| `src/canvas/mod.rs` | `ToolKind`, `Annotation`, `Canvas` |
| `src/canvas/render.rs` | tiny-skia + text + pixelate |
| `src/export/clipboard.rs` | `wl-copy` + latest file |
| `src/export/file.rs` | PNG save |
| `src/tray/sni.rs` | StatusNotifierItem |
| `README.md` | User-facing contract (verify against code) |
| `Cargo.toml` | Binary `rustshot-wayland`, deps, release abort |
