# Revision of cp-261124: General Comments - Round 2

Date: 2026-11-28

## Comments

1. Question: do `{win, mac}/{doubao, mingkuai}.rs` duplicate `protocol/{doubao, mingkuai}.rs`?
2. Request: the top-level `enable` setting takes either an array or the string `"all_available"`, which means all supported input methods available on the system.

Asked with `AskUserQuestion` before the work:

- Default of `enable`: the user chose `"all_available"`.
- Sharing the pipe code of `win/doubao.rs` and `win/mingkuai.rs` (see the answer to 1): the user chose to share it in `win/pipe.rs`.

## Answers

1. No. `protocol/doubao.rs` and `protocol/mingkuai.rs` only build and parse the bytes of the messages (no I/O, unit-tested on every platform). The platform files only move those bytes: `win/doubao.rs` and `win/mingkuai.rs` open the named pipe, write the request from `protocol::*::*_request()` and hand the reply to `protocol::*::parse_*()`. `mac/mingkuai.rs` does the same over a Unix socket. There is no shared code between a platform file and its protocol file. There was duplication between `win/doubao.rs` and `win/mingkuai.rs`, though: the same `Pipe` handle type with its `Drop`, the same `open` that waits once on `ERROR_PIPE_BUSY`, and nearly the same `read_exact` (the Doubao one fails on a short read, the Mingkuai one loops until the buffer is full). That is moved to `win/pipe.rs` (see "Done").

## To do

- [x] `src/win/pipe.rs`: the shared pipe code of `src/win/doubao.rs` and `src/win/mingkuai.rs`.
- [x] `src/config.rs`: `enable` takes a list or `"all_available"`, the default is `"all_available"`. Names are checked when the file is read.
- [x] `src/win/ime.rs`, `src/mac/ime.rs`, `src/mac/ffi.rs`, `src/cli.rs`: find the supported input methods that the system has installed, and use them for `"all_available"`.
- [x] `src/config.rs`: hotkeys follow the enabled set that `cli` applies at start.
- [x] `config.example.toml`, `docs/configuration.md`: describe the two forms and the new default.

## Done

- `2fb1567`: `src/win/pipe.rs` holds the pipe handle (closed on drop), the open that waits once on a busy server, `write` and `read_exact`. `src/win/doubao.rs` and `src/win/mingkuai.rs` keep only their message calls. Doubao replies are now read until the buffer is full, as Mingkuai's already were, instead of failing on a short read.
- `de6b99e`: `enable` is a `config::Enable`: `AllAvailable` (the default) or `List`. Its names are checked when the file is read, and a wrong word or type gets an error that names both forms. `cli::read_global_settings` resolves it with `platform::available_imes()` and calls `Ime::set_enabled`. `Config::shortcut` now asks `Ime::is_enabled` instead of reading the list itself, so the hotkeys of both platforms follow the resolved set without a change in `win/hotkeys.rs` or `mac/hotkeys.rs`. "Available" is:
  - Windows (`win::ime::available`): US when an English (US) layout is in the keyboard layout list (`GetKeyboardLayoutList`), Pinyin and Doubao when their TSF profiles for zh-CN have `TF_IPP_FLAG_ENABLED`. When TSF cannot be asked, both count as available.
  - macOS (`mac::ime::available`, `ffi::enabled_sources`): US, Doubao and Shuangpin when their input source is enabled (`TISCreateInputSourceList` without the installed-only ones). When the list cannot be read, all three count.
  - Other platforms: all four.
  - `config.example.toml` and `docs/configuration.md` describe both forms, the new default and that adding an input method to the system needs `imetap daemon reload`.
- `review.md`: F1 and C2 mention the new form. C2 also points at the current files and commands, and checks `"all_available"`.

## Not as asked

None

## Checks

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets` for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu` and `aarch64-apple-darwin`: no warnings.
- `cargo test`: 67 passed. The case of `enable` in `config::tests` now covers `"all_available"`, a list, and the wrong word, name and type.
- Not run on Windows or macOS: the TSF and TIS queries behind `"all_available"` are checked to compile only. C2 in `review.md` has the steps to try on Windows.
