# Revision of cp-261124: General Comments - Round 1

Date: 2026-11-28

## Comments

- Rename the daemon binary `imetap` to `imetapd` and the CLI `imetapctl` to `imetap`, one CLI for Windows and macOS.
- Group the commands: `daemon` (`run`, `start`, `stop`, `reload`, no `restart`), `switch` (`im`, `lang`), new `paths`, new `init`, `config` with only `edit` (`$EDITOR`, or the default GUI editor with `--gui`), no `help` command.

## Answers

None (no questions).

## To do

- [x] `src/main.rs` becomes the CLI (`imetap`, console subsystem), `src/bin/imetapd.rs` the daemon (GUI subsystem on Windows), delete `src/bin/imetapctl.rs`.
- [x] `src/cli.rs`: new command tree, `reload`, `paths`, `init`, `config edit [--gui]`.
- [x] `src/lib.rs`, `src/win/autostart.rs`, `src/mac/mod.rs`, `scripts/*.py`: daemon name `imetapd`, login entries run it without `run`.
- [x] Docs: `README.md`, `CLAUDE.md`, `config.example.toml`, comments and messages.

## Done

- Code, scripts and login entries: see the first commit after "Start revision". `imetap` is the console CLI (`src/main.rs`), `imetapd` the daemon (`src/bin/imetapd.rs`, takes only `--debug`), `imetapctl` is gone.
- `imetap paths` prints the config file, the debug log and the daemon executable. It notes when the running daemon does not write the debug log (it is the only log, and only with `--debug` or `IMETAP_DEBUG`).
- Docs: `README.md`, `CLAUDE.md`, `config.example.toml`, `review.md`.

## Not as asked

- `daemon reload` checks the config file, then stops and starts `imetapd` again. It does not reload inside the running process, because hooks, hotkeys, the tap detector and the tray read the config only at start, and the code cannot be run here (Linux) or on a Mac to test an in-process version. A broken config is reported and the running daemon stays.
- `config` keeps one subcommand, `edit`, as asked. Without `--gui` and without `$EDITOR`, it fails with a message that names `--gui`.
- Old login entries that pass `run` to the program (and point at `imetap`) stop working. Run `imetap autostart enable` again. No compatibility is kept, the project is 0.x.

## Checks

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (67 passed) on Linux.
- `cargo check --all-targets` for `aarch64-apple-darwin` and `x86_64-pc-windows-gnu`: clean. Not run on Windows or macOS, so `daemon start|stop|reload`, `autostart` and the bundle in `scripts/install.py` are untested.
- By hand: `--help`, `daemon` without a subcommand (shows its help), `paths`, `config edit` without `$EDITOR`.
