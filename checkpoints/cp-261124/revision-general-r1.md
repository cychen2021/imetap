# Revision of cp-261124: General Comments - Round 1

Date: 2026-11-28

## Comments

- Rename the daemon binary `imetap` to `imetapd` and the CLI `imetapctl` to `imetap`, one CLI for Windows and macOS.
- Group the commands: `daemon` (`run`, `start`, `stop`, `reload`, no `restart`), `switch` (`im`, `lang`), new `paths`, new `init`, `config` with only `edit` (`$EDITOR`, or the default GUI editor with `--gui`), no `help` command.

## Answers

None (no questions).

## To do

- [ ] `src/main.rs` becomes the CLI (`imetap`, console subsystem), `src/bin/imetapd.rs` the daemon (GUI subsystem on Windows), delete `src/bin/imetapctl.rs`.
- [ ] `src/cli.rs`: new command tree, `reload`, `paths`, `init`, `config edit [--gui]`.
- [ ] `src/lib.rs`, `src/win/autostart.rs`, `src/mac/mod.rs`, `scripts/*.py`: daemon name `imetapd`, login entries run it without `run`.
- [ ] Docs: `README.md`, `CLAUDE.md`, `config.example.toml`, comments and messages.

## Done

## Not as asked

## Checks
