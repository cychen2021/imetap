# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# imetap

A small tray tool, written in Rust, for people who type Chinese and English on Windows or macOS: it switches input methods with global hotkeys, toggles Chinese/English with a Ctrl double-tap, and shows a translucent HUD whenever the input method or its mode changes, including when focus moves to a window whose input method or mode differs.

## Motivation

It replaces two personal scripts (AutoHotkey `hotkeys.ahk` on Windows, Hammerspoon `ime.lua` on macOS) and an unmaintained project, IMEInd, which only showed an indicator. On Windows the input method state is per window and the Microsoft Pinyin mode cannot be queried or set through any documented API in TSF-native apps, which shapes the design (see `docs/architecture.md`).

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

Commands of `imetap`: `daemon run [--debug]|start|stop|reload`, `switch im <ime>|lang`, `status [-v]`, `autostart enable [--debug]|disable|status`, `paths`, `init`, `config edit [--gui]`. There is no `help` command, only `--help`. `daemon reload` checks the config file and then stops and starts the daemon again (it does not reload inside the running process). The daemon `imetapd` takes only `--debug`.

Detailed user documentation is in `docs/` (`index.md` links the pages). README.md keeps the overview and links there. Update the matching page when a command, a config key or the install steps change.

## Architecture

`cli.rs` is the clap CLI of `imetap` (`src/main.rs`) and the entry of the resident process `imetapd` (`src/bin/imetapd.rs`). It delegates to `imetap::win` (`src/win/`) or `imetap::mac` (`src/mac/`). The platform-independent modules (`tap`, `hold`, `debug`, `fade`, `reminder`, `state`, `doubao`, `mingkuai`, `config`) are unit-tested without touching the OS. [`docs/architecture.md`](docs/architecture.md) describes each module and how Windows and macOS differ. Read the part for the module you change.

Rules that the code depends on:

- All `unsafe` FFI is confined to `src/win/` and `src/mac/ffi.rs`, each call with a `SAFETY` comment.
- Windows: never hard-code the Microsoft Pinyin toggle hotkey. `win/hotkey.rs` reads it from `HKCU\Control Panel\Input Method\Hot Keys\00000010` (default Ctrl+Space).
- Colours in `config.toml` are names, never RGB.
- Slow work (UI Automation, pipes, `SendInput`) never runs on the Windows message thread, and the hook procedures stay trivial.

Hidden Mingkuai support (not in `docs/`, the README, `config.example.toml` or help text):

- `mingkuai.rs`: the hidden Mingkuai support (Mingkuai is the user's own input method, with the server `mingkuaid`). It is off unless `config.toml` has `[mingkuai-integrate]` with `enable = true`, and it also takes `hotkey`, `color` and `label` there. Keep it out of the README, `config.example.toml` and help text. `cli::main` reads the flag for every command. When it is off, Mingkuai is `Ime::Other`. The module builds and parses the messages of the `mingkuaid` protocol (a 4-byte little-endian length, then JSON: `"get_mode"`, `{"set_mode":{"chinese":...}}`, reply `{"mode":{"chinese":...}}`). The mode is one mode for all applications. `win/mingkuai.rs` does the pipe I/O on `\\.\pipe\mingkuai-<user name>`, and `win/ime.rs` identifies Mingkuai by its fixed CLSID. On macOS, `mac/mingkuai.rs` does the socket I/O on `~/Library/Application Support/Mingkuai/mingkuaid.sock` (`mingkuai::socket_path`), and `mac/mod.rs` identifies Mingkuai by its input mode `org.mingkuai.inputmethod.Mingkuai.Pinyin`. The Ctrl double-tap in Mingkuai sets its mode and stays in the same input source. `mac/app.rs` keeps a `"WatchMode"` connection on its own thread and wakes the main thread with `dispatch_async_f` on each change. Mode changes made inside Mingkuai (for example with Shift) cause no system event, so `win/app.rs` keeps a `"WatchMode"` connection open on its own thread (it tries again every 5 seconds while the server is down). Each change becomes `Job::MingkuaiMode`, which shows the HUD when Mingkuai is the active input method.

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
