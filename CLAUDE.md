# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# imetap

A small tray tool, written in Rust, for people who type Chinese and English on Windows or macOS: it switches input methods with global hotkeys, toggles Chinese/English with a Ctrl double-tap, and shows a translucent HUD whenever the input method or its mode changes, including when focus moves to a window whose input method or mode differs.

## Motivation

It replaces two personal scripts: an AutoHotkey script (`hotkeys.ahk`) on Windows and a Hammerspoon script (`ime.lua`) on macOS. It also replaces an older, unmaintained GitHub project, IMEInd, which only showed an indicator. On Windows the input method state is per window and the Microsoft Pinyin mode cannot be queried or set through any documented API in TSF-native apps, which shapes the design below.

## Status

Windows is implemented and tested by hand. macOS (`src/mac/`) is a port of `ime.lua` that compiles (`cargo check --target aarch64-apple-darwin`) but has never been run; treat it as untested. `AGENTS.md` is a symlink to this file.

## Commands

```bash
cargo build                                  # build
cargo run -- <args>                          # run a subcommand, e.g. `cargo run -- status -v`
cargo test                                   # all tests
cargo test <name_substring>                  # tests whose name contains the substring
cargo clippy --all-targets                   # lint
cargo clippy --target aarch64-apple-darwin   # lint the macOS code from Windows (compile check only)
cargo fmt                                    # format
```

Subcommands: `run` (foreground; the default), `start`, `stop`, `restart`, `set <ime>`, `toggle`, `status [-v]`, `autostart enable|disable|status`, `config path|init|edit|check`. The release binary on Windows is a GUI-subsystem executable that attaches to the parent console for the management commands, so `println!` output only shows up in a terminal, not when the output is piped through `cmd /c`.

## Architecture

`main.rs` is a clap CLI that delegates to a `platform` module (`imetap::win` or `imetap::mac`). The platform-independent logic is unit-tested without touching the OS:

- `tap.rs`: the Ctrl double- and triple-tap detector. A tap is a Ctrl press shorter than `hold_ms` with no other key or mouse button in between; two taps within `gap_ms` fire. A third tap within `triple_ms` makes a triple tap (show the HUD), so the double tap is held back until a timer of `triple_ms` expires or another key or click resolves it. Timestamps come from the events themselves.
- `reminder.rs`: the optional reminder. `resume` mode shows the HUD when the user comes back after `idle_secs` without key, mouse or focus-window activity; `resume-and-switch` also shows it when leaving a window used for `dwell_secs` without a HUD in between. Wall-clock timestamps, so sleep counts as idle.
- `state.rs`: `Ime`, and `Snapshot` (input method plus optional Chinese/English mode) with its glyph, caption and HUD colours (red for "中", blue for "EN", grey for "?").
- `doubao.rs`: the wire format of the Doubao IME's named pipe (`\\.\pipe\ObricIme\oime-server`).
- `config.rs`: `config.toml` from `$XDG_CONFIG_HOME/imetap/`, else the platform default; unknown keys are errors.

Windows (`src/win/`), everything event-driven with no polling:

- `app.rs`: one message-loop thread owns the hidden window, tray icon, `RegisterHotKey` hotkeys, the `WH_KEYBOARD_LL`/`WH_MOUSE_LL` hooks (kept trivial; they ignore injected keys, which also covers our own `SendInput`) and two `SetWinEventHook`s (foreground change, IME change). The hooks also feed the reminder, which posts a message when it is due; the next debounced read then always shows the HUD. Notifications are debounced 250 ms, then a worker thread reads the state; slow work (UI Automation, the pipe, `SendInput`) never runs on the message thread. The worker reports back by posting a message.
- `ime.rs`: TSF `ITfInputProcessorProfileMgr` to activate and identify Pinyin and Doubao by CLSID; the US layout is requested from the foreground window with `WM_INPUTLANGCHANGEREQUEST`.
- `doubao.rs`: Doubao's mode is read and set over its named pipe (ops 13/14); the server also updates its own status icon.
- `hotkey.rs`: Microsoft Pinyin's mode lives in the engine inside `ctfmon.exe`; IMM messages, TSF compartments, `ImmSimulateHotKey` and the language bar all fail in TSF-native apps (new Notepad, Edge). The only thing that works is the system IME toggle hotkey, whose combination is read from `HKCU\Control Panel\Input Method\Hot Keys\00000010` (default Ctrl+Space) and sent with `SendInput`. Do not hard-code it.
- `uia.rs`: Pinyin's current mode is read from the taskbar's "Tray Input Indicator" button via UI Automation (names like "Chinese Mode"; English and Chinese Windows are both handled). It is slow when run cold, hence the cached client on the worker thread.
- `hud.rs`: a layered, topmost, no-activate, click-through window on the monitor of the foreground window (primary if none), bottom centre, faded out by timers.
- `autostart.rs`: the `HKCU\...\Run` value.

macOS (`src/mac/`), a port of `ime.lua`: the double-tap swaps the US source and the last-used Chinese source (persisted next to the config); there is no mode to read, because Doubao and Shuangpin are separate input sources. A Quartz event tap (needs Accessibility) handles the double-tap and the hotkeys, and a second, listen-only tap watches mouse movement for the reminder; the HUD is an `NSPanel`, the tray an `NSStatusItem`; focus and input-source changes come from `NSWorkspace` and the distributed notification `com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged`. The login item is a LaunchAgent plist.

All `unsafe` FFI is confined to `src/win/` and `src/mac/ffi.rs`, each call with a `SAFETY` comment.

## Conventions (from vischat)

- Error handling: `anyhow` with `Result<T>`. CLI parsing: `clap` with the `derive` feature. Rust edition 2024.
- Put unit tests in the same file as the code, and integration and CLI tests in `tests/`.
- Prefer a `use` import to an absolute path at the point of use. Write `use crate::foo::Bar;` and then `Bar`, not `crate::foo::Bar` inline.
- Keep each `use` group to a single brace level. A flat list is fine (`use crate::model::{A, B};`), but put nested paths on their own lines.
- Add doc comments to public APIs.
- Do not hard-wrap lines in Markdown docs (README.md, this file). Write each paragraph or list item on one line.
- `x_*` and `tmp/` are scratch paths and should be gitignored, so don't commit them.
- Commit messages: in English. The headline has fewer than 10 words, starts with a capitalized verb and has no trailing period. Details go in a bullet list after one blank line. Wrap code items and file paths in backticks. Put the most important change first. Avoid vague verbs ("refactor", "enhance", "improve") when a more precise one exists. Describe moves and renames as moves and renames, not as a deletion plus an addition. A minor change gets a one-line message.
