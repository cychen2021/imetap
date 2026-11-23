# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# imetap

A small tray tool, written in Rust, for people who type Chinese and English on Windows or macOS: it switches input methods with global hotkeys, toggles Chinese/English with a Ctrl double-tap, and shows a translucent HUD whenever the input method or its mode changes, including when focus moves to a window whose input method or mode differs.

## Motivation

It replaces two personal scripts: an AutoHotkey script (`hotkeys.ahk`) on Windows and a Hammerspoon script (`ime.lua`) on macOS. It also replaces an older, unmaintained GitHub project, IMEInd, which only showed an indicator. On Windows the input method state is per window and the Microsoft Pinyin mode cannot be queried or set through any documented API in TSF-native apps, which shapes the design below.

## Status

Windows is implemented and tested by hand. macOS (`src/mac/`) is a port of `ime.lua` that compiles (`cargo check --target aarch64-apple-darwin`) but has never been run. Treat it as untested. `AGENTS.md` is a symlink to this file.

## Commands

```bash
cargo build                                  # build
uv run scripts/build.py [--check]            # release build, lists the binaries (--check: fmt, clippy, tests first)
uv run scripts/install.py                    # cargo install, plus the signed app bundle on macOS
cargo run -- <args>                          # run a subcommand, e.g. `cargo run -- status -v`
cargo test                                   # all tests
cargo test <name_substring>                  # tests whose name contains the substring
cargo clippy --all-targets                   # lint
cargo clippy --target aarch64-apple-darwin   # lint the macOS code from Windows (compile check only)
cargo fmt                                    # format
```

Subcommands: `run [--debug]` (foreground, the default), `start`, `stop`, `restart`, `set <ime>`, `toggle`, `status [-v]`, `autostart enable [--debug]|disable|status`, `config path|init|edit|check`. The release binary on Windows is a GUI-subsystem executable that attaches to the parent console for the management commands, so `println!` output only shows up in a terminal, not when the output is piped through `cmd /c`.

## Architecture

`cli.rs` is a clap CLI that delegates to a `platform` module (`imetap::win` or `imetap::mac`). Two thin binaries call it: `main.rs` (`imetap`, GUI subsystem on Windows, attaches to the parent console) and `src/bin/imetapctl.rs` (console subsystem, so output works in pipes, and it prints help without a command). On non-Windows platforms `imetapctl` is a stub that points to `imetap`. `daemon_exe()` in `lib.rs` makes `imetapctl` start and register the `imetap` next to it. The platform-independent logic is unit-tested without touching the OS:

- `tap.rs`: the Ctrl double- and triple-tap detector. A tap is a Ctrl press shorter than `hold_ms` with no other key or mouse button in between. Two taps within `gap_ms` fire. A third tap within `triple_ms` makes a triple tap (show the HUD), so the double tap is held back until a timer of `triple_ms` expires or another key or click resolves it. Timestamps come from the events themselves.
- `hold.rs`: the queue behind `tap.hold_input` (Windows only). From a double tap until the toggle has been applied, the keyboard hook swallows every key event (Ctrl included, so a Ctrl+key shortcut stays intact) and the worker replays them in order with `SendInput`, so keys typed during the wait for a third tap or while the input method switches reach the application in the new mode. A timeout releases it if the toggle never reports back.
- `debug.rs`: the debug log. `run --debug` (which `autostart enable --debug` puts in the login entry) or `IMETAP_DEBUG=1` at start makes the resident process append one line per event (the state it reads, the checks and the actions) to `imetap-debug.log` in the temporary directory, starting over past 2 MB. `status` shows whether the running instance does. It cannot read another process's environment, so it asks the instance: through a registered window message on Windows, and through a second line in the instance lock file on macOS.
- `fade.rs`: the HUD's brightness over time (fade in, hold, fade out) as a pure function of the elapsed time, shared by both platforms.
- `reminder.rs`: the optional reminder. `resume` mode shows the HUD when the user comes back after `idle_secs` without key, mouse or focus-window activity. `resume-and-switch` also shows it when leaving a window used for `dwell_secs` without a HUD in between. Wall-clock timestamps, so sleep counts as idle.
- `state.rs`: `Ime`, and `Snapshot` (input method plus optional Chinese/English mode) with its glyph and caption (the caption only appears in `status` output), and `Color`, the named colours (each with a deep shade for backgrounds and a bright one for stripes) that `config.toml` chooses from: `hud.style()` gives the HUD a background that follows the mode (`[hud.colors]`, red for "中", blue for "EN"), a left stripe in the input method's colour (`[hud.ime_colors]`) with its big label (`[hud.ime_labels]`, e.g. "豆" for Doubao), and, for an input method without a mode, its own colour as the background. An input method imetap does not know (`Ime::Other`) carries the name the system gives it in the `Snapshot` (interned, so that it stays `Copy`, and read from the TSF profile description or the language name on Windows and from the TIS localized name on macOS), and `label_for_name`/`color_for_name` derive its label and colour from it. Colours are names, never RGB.
- `doubao.rs`: the wire format of the Doubao IME's named pipe (`\\.\pipe\ObricIme\oime-server`).
- `mingkuai.rs`: the hidden Mingkuai support (Mingkuai is the user's own input method, with the server `mingkuaid`). It is off unless `config.toml` has `[mingkuai-integrate]` with `enable = true`, and it also takes `hotkey`, `color` and `label` there. Keep it out of the README, `config.example.toml` and help text. `cli::main` reads the flag for every command. When it is off, Mingkuai is `Ime::Other`. The module builds and parses the messages of the `mingkuaid` protocol (a 4-byte little-endian length, then JSON: `"GetMode"`, `{"SetMode":{"chinese":...}}`, reply `{"Mode":{"chinese":...}}`). The mode is one mode for all applications. `win/mingkuai.rs` does the pipe I/O on `\\.\pipe\mingkuai-<user name>`, and `win/ime.rs` identifies Mingkuai by its fixed CLSID. On macOS, `mac/mingkuai.rs` does the socket I/O on `~/Library/Application Support/Mingkuai/mingkuaid.sock` (`mingkuai::socket_path`), and `mac/mod.rs` identifies Mingkuai by its input mode `org.mingkuai.inputmethod.Mingkuai.Pinyin`. The Ctrl double-tap in Mingkuai sets its mode and stays in the same input source. `mac/app.rs` keeps a `"WatchMode"` connection on its own thread and wakes the main thread with `dispatch_async_f` on each change. Mode changes made inside Mingkuai (for example with Shift) cause no system event, so `win/app.rs` keeps a `"WatchMode"` connection open on its own thread (it tries again every 5 seconds while the server is down). Each change becomes `Job::MingkuaiMode`, which shows the HUD when Mingkuai is the active input method.
- `config.rs`: `config.toml` from `$XDG_CONFIG_HOME/imetap/`, else the platform default. Unknown keys are errors.

Windows (`src/win/`), everything event-driven with no polling:

- `app.rs`: one message-loop thread owns the hidden window, tray icon, `RegisterHotKey` hotkeys, the `WH_KEYBOARD_LL`/`WH_MOUSE_LL` hooks (kept trivial, and they ignore injected keys, which also covers our own `SendInput`) and two `SetWinEventHook`s (foreground change, IME change). The hooks also feed the reminder, which posts a message when it is due. The next debounced read then always shows the HUD. Notifications are debounced 250 ms, then a worker thread reads the state. Slow work (UI Automation, the pipe, `SendInput`) never runs on the message thread. The worker reports back by posting a message.
- `ime.rs`: TSF `ITfInputProcessorProfileMgr` to activate and identify Pinyin and Doubao by CLSID. The US layout is requested from the foreground window with `WM_INPUTLANGCHANGEREQUEST`.
- `doubao.rs`: Doubao's mode is read and set over its named pipe (ops 13/14). The server also updates its own status icon.
- `hotkey.rs`: Microsoft Pinyin's mode lives in the engine inside `ctfmon.exe`. IMM messages, TSF compartments, `ImmSimulateHotKey` and the language bar all fail in TSF-native apps (new Notepad, Edge). The only thing that works is the system IME toggle hotkey, whose combination is read from `HKCU\Control Panel\Input Method\Hot Keys\00000010` (default Ctrl+Space) and sent with `SendInput`. Do not hard-code it.
- `uia.rs`: Pinyin's current mode is read from the taskbar's "Tray Input Indicator" button via UI Automation (names like "Chinese Mode", with English and Chinese Windows both handled). It is slow when run cold, hence the cached client on the worker thread. The same client tells whether the focus is in a text field (`hud.wait_for_text_field`). Only text fields count. The HUD shows when focus is in a text field and the state differs from the state at the last text field (`state::text_field_change`), so passing through windows without one, or coming back to the same Chinese text box, shows nothing. `is_text_field` accepts a writable `Edit` or value element, or one with the text pattern, and rejects a browser page, which is a read-only `Document`. A bare custom-drawn window such as WezTerm tells nothing and counts as text. In a Chromium window (class `Chrome_WidgetWin_*`: Chrome, Edge, Electron) a bare window means the accessibility tree is not ready yet, so `Engine::focus_in_text_field` asks again a few times and then counts it as not text. Any UIA failure also counts as text. While the last check found no text field, focus events, key presses and clicks trigger a new check. `status -v` prints what UIA reports for the focus. With `IMETAP_DEBUG=1`, the checks are in the debug log (see `debug.rs`).
- `hud.rs`: a layered, topmost, no-activate, click-through window on the monitor of the foreground window (primary if none), bottom centre, faded in and out by a 15 ms timer that asks `fade.rs` for the brightness (`hud.fade_in_ms`, `hud.duration_ms`, `hud.fade_out_ms`). Showing again while visible carries on from the current brightness. macOS steps the same curve with one-shot run loop timers, one chain at a time.
- `autostart.rs`: the `HKCU\...\Run` value.

macOS (`src/mac/`), a port of `ime.lua`: the double-tap swaps the US source and the last-used Chinese source (persisted next to the config). There is no mode to read, because Doubao and Shuangpin are separate input sources. A Quartz event tap (needs Accessibility) handles the double-tap and the hotkeys, and a second, listen-only tap watches mouse movement for the reminder. The HUD is an `NSPanel`, the tray an `NSStatusItem` whose template icon shows "中", or "E" while the state is English (the US source or an English mode, `app::set_last`). Focus and input-source changes come from `NSWorkspace` and the distributed notification `com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged`. The login item is a LaunchAgent plist. `scripts/install.py` builds `~/Applications/imetap.app` (`LSUIElement`, the icon from `assets/mac/`) around the installed binary and signs the bundle with a self-signed certificate, and `daemon_exe()` prefers the bundle's executable (`MAC_BUNDLE_EXE`), because the Accessibility permission belongs to the bundle. `assets/mac/` has the macOS-specific artwork: `icon.svg` (the app icon, on the macOS grid) and `menubar.svg` (an outlined pill drawn as a template image, shown when `tray.enable_lang_indicator` is false). `assets/` has the Windows ones: `logo.svg`, `tray.png`, `icon.ico`.

All `unsafe` FFI is confined to `src/win/` and `src/mac/ffi.rs`, each call with a `SAFETY` comment.

## Conventions (from vischat)

- Error handling: `anyhow` with `Result<T>`. CLI parsing: `clap` with the `derive` feature. Rust edition 2024.
- Put unit tests in the same file as the code, and integration and CLI tests in `tests/`.
- Prefer a `use` import to an absolute path at the point of use. Write `use crate::foo::Bar;` and then `Bar`, not `crate::foo::Bar` inline.
- Keep each `use` group to a single brace level. A flat list is fine (`use crate::model::{A, B};`), but put nested paths on their own lines.
- Add doc comments to public APIs.
- Do not use semicolons or em-dashes in help messages, docs, doc comments and comments. Write two sentences, or use a comma or a colon.
- Do not hard-wrap lines in Markdown docs (README.md, this file). Write each paragraph or list item on one line.
- `x_*` and `tmp/` are scratch paths and should be gitignored, so don't commit them.
- Commit messages: in English. The headline has fewer than 10 words, starts with a capitalized verb and has no trailing period. Details go in a bullet list after one blank line. Wrap code items and file paths in backticks. Put the most important change first. Avoid vague verbs ("refactor", "enhance", "improve") when a more precise one exists. Describe moves and renames as moves and renames, not as a deletion plus an addition. A minor change gets a one-line message.
