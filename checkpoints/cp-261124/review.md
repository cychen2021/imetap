# Checkpoint cp-261124

Range: `2870fc1..af102bc` (44 commits, 47 files, the whole history). Previous: none.
Checks: `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test` (67 tests) and `cargo check --target aarch64-apple-darwin` pass.

## What changed

- Windows core: tray, hotkeys, Ctrl double and triple tap, HUD with fade, input method and mode reading (Microsoft Pinyin through UI Automation and the system toggle hotkey, Doubao through its pipe).
- Text field rule: on a window switch the HUD waits for a text field (`src/win/uia.rs`, `src/state.rs`), with Chromium and Explorer special cases.
- Key hold: keys typed during a double tap are held and replayed (`src/hold.rs`).
- Reminder: HUD after idling or after a long stay in one window (`src/reminder.rs`).
- macOS port (`src/mac/`): event taps, HUD panel, status item, login item, signed app bundle from `scripts/install.py`. Compiles only, never run.
- Config: `enable` list, `[tray]`, `[hud.ime_labels]` and `[hud.ime_colors]`, labels and colours derived for unknown input methods.
- Hidden Mingkuai support on both platforms (`src/mingkuai.rs`, `src/win/mingkuai.rs`, `src/mac/mingkuai.rs`).
- Tooling: debug log, `imetapctl`, `scripts/build.py`, `scripts/install.py`, logo and icon assets, Claude skills submodule.

## Problems found

These need your decision.

F1. `config.example.toml:6`, `src/config.rs`: config key names are not consistent. A section switch is `enabled` (`tap.enabled`, `hud.enabled`, `reminder.enabled`), but `tray.enable_lang_indicator` and `mingkuai-integrate.enable` use `enable`, the top level `enable` is a list of input methods, and the Mingkuai section name is the only one with a hyphen (`mingkuai-integrate`, others are `snake_case`). Options: (a) rename to `tray.lang_indicator`, `mingkuai_integrate.enabled` and the top level `imes` (config format change, the project has no compatibility promise yet), (b) rename only the Mingkuai section to `mingkuai_integrate` and keep the rest, (c) leave it. Recommended: (a), because the config format is public and is cheapest to fix now.

## Minor problems

I fix these after your approval ("approve all", or the numbers to skip or change).

- M1. `src/cli.rs:362`, `src/cli.rs:364`, `src/cli.rs:374`, `src/win/app.rs:189`: inline `crate::mac::...`, `crate::debug::...`, `crate::mingkuai::...` paths. Fix: `use` them.
- M2. `src/cli.rs:238`, `src/win/app.rs:915`, `src/mac/app.rs:192`: the "create the default config file if missing, then open it" steps are written three times. Fix: one `config::ensure_file()` returning the path, used by all three.
- M3. `src/cli.rs:338`, `src/win/app.rs:923`: the result of `ShellExecuteW` is ignored, so `config edit` and the tray entry "Open config file" report success when the shell refused (a value of 32 or less is an error). The call is also written twice. Fix: one `win::open` that checks the result, used by both.
- M4. `src/cli.rs:316`, `src/cli.rs:361`: `debug_status` is written twice, and the two copies differ only in the call that asks the running instance. Fix: one function in `src/cli.rs` that takes the answer (`Option<bool>`).
- M5. `README.md:48`: the config block says it is the same as `config.example.toml`, but it has no `tap.hold_input` and no `[tray]` section. Fix: add both.
- M6. `src/win/app.rs:949`: `let show = APP.with(...)` holds `changed` (whether the tray icon must be updated), not whether the HUD shows. Fix: rename to `changed`.
- M7. `src/win/app.rs:468`: the keyboard and mouse hooks are installed only when `tap.enabled` or `reminder.enabled`. With both off and `hud.wait_for_text_field` on, the "key press or click triggers a new text field check" rule of `CLAUDE.md` never fires, because it lives in those hooks. Fix: also install them when `wait_for_text_field` is on.
- M8. `src/mac/app.rs:666`: `schedule` creates a run loop timer and never releases the creator's reference, so every debounce and triple tap timer leaks one object (the comment at `schedule_probe` (`src/mac/app.rs:651`) says the run loop releases it, which is only the run loop's own reference). Fix: release the reference after the timer fired or was invalidated.
- M9. `src/config.rs:661`, `src/config.rs:646`: tests that mostly check serde (`unknown = 1` is rejected, a default is the default, already covered by `the_template_spells_out_the_defaults` and `unknown_keys_are_rejected`). Fix: keep only the assertions that are not covered (the Mingkuai shortcut and label) and delete the rest, by `trim-tests`.

## Needs your check

I could not settle these. Each needs a run on your side.

C1. `src/mac/` (all of it): the macOS port has never run, and it has gained many features since the first port (Mingkuai, tray "E", signed bundle, reminder, hotkeys swallowed in the event tap). Check on a Mac: `uv run scripts/install.py`, grant Accessibility, `imetap start`, then (1) press `Ctrl+Shift+S` and `Ctrl+Shift+F` and watch the HUD and the menu bar icon, (2) double-tap `Ctrl` and see the US and Shuangpin sources swap, (3) switch source from the menu bar and see the HUD, (4) set `reminder.enabled = true` with a small `idle_secs`, stop touching the mouse and keyboard, move the mouse and see the HUD, (5) `imetap restart` and `imetap status -v`. A problem if: a step shows nothing, the shortcut also types a character, the HUD appears twice, or Accessibility is asked for again after a reinstall. Then: I fix what shows up, one problem at a time.
C2. `src/win/app.rs:706`, `src/config.rs:398` (`enable`, commit `daa57f9`): the `enable` list has unit tests, but nothing runs it on Windows. Check: set `enable = ["us", "doubao"]`, `imetap restart`, press `Ctrl+Shift+F` (it must do nothing), switch to Microsoft Pinyin from the language bar, and run `imetap set pinyin` (it must say unknown input method). A problem if: `Ctrl+Shift+F` still switches, or the HUD for Pinyin shows a wrong label or no label. Then: I fix the hotkey registration or the unknown name path.

## Past concerns, checked again

No earlier review exists. Checked against `CLAUDE.md` and the usual list:

- Writing rules (no semicolons or em dashes in comments, docs and help text): none found.
- `use` rules: M1.
- Duplicated code and docs out of date: M2, M3, M4, M5.
- Names that carry no information or mean something else: M6, F1.
- Tests that check a library: M9.
- File size: `src/win/app.rs` (1111 lines) and `src/mac/app.rs` (1032 lines), see Where to Look - 2.

## Where to look

### Where to Look - 1: What to do with the untested macOS port

What: `CLAUDE.md` says macOS "compiles but has never been run", yet about a third of the recent commits (Mingkuai, tray, bundle, icons, hotkey changes) are macOS only, all written without running.
Why now: each further macOS commit builds on code nobody has seen work, so one wrong base assumption (for example how the event tap sees Ctrl) can invalidate many of them.
Options: (a) run C1 now and freeze macOS features until it passes, (b) keep going and test later, (c) drop macOS from the README promise until it runs. Recommended: (a), the cost is one session on a Mac.
Scope: all of `src/mac/` (about 1650 lines), `scripts/install.py`. Entry points: `src/mac/app.rs`, `src/mac/mod.rs`.

### Where to Look - 2: Two large app files with the same shape

What: `src/win/app.rs` and `src/mac/app.rs` each hold the whole resident process, and repeat the same logic: the show-the-HUD decision (`changed && !first && on_window_switch`), the Mingkuai watch loop with a 5 second retry, the reminder wiring and the debounce.
Why now: every new feature (reminder, Mingkuai, tray indicator) was written twice, and the two copies already differ in small ways (for example the text field rule only exists on Windows).
Options: (a) move the platform independent decisions into pure, tested functions (the show decision into `src/state.rs`, the watch loop into `src/mingkuai.rs` taking a callback) and keep the OS calls in place, (b) split each file by job (hooks, tray, messages) without changing logic, (c) leave it. Recommended: (a), it is small and gives tests for logic that is hard to see wrong today.
Scope: 2 large files plus `src/state.rs` and `src/mingkuai.rs`. Entry points: `src/win/app.rs:949`, `src/mac/app.rs:546`, `src/win/app.rs:251`, `src/mac/app.rs:570`.

### Where to Look - 3: Hidden Mingkuai support spreads through the code

What: Mingkuai is meant to stay out of the README and help, but it is now in 9 files, in a global switch set by `cli::main` (`mingkuai::set_enabled`), in `Ime` and in the hotkey list of both platforms.
Why now: a second private input method, or a request to publish Mingkuai, would force another round of edits in all of these places.
Options: (a) leave it, (b) turn the private input method into a general "custom input method" entry in the config (name, hotkey, pipe or socket, label), (c) move all Mingkuai code under one `mingkuai/` directory. Recommended: (a) for now, and (c) if the code grows again.
Scope: `src/mingkuai.rs`, `src/win/mingkuai.rs`, `src/mac/mingkuai.rs`, `src/win/ime.rs`, `src/mac/mod.rs`, `src/config.rs`, `src/state.rs`, `src/cli.rs`.

### Where to Look - 4: `CLAUDE.md` is the only design document

What: `CLAUDE.md` holds the whole architecture, about 60 long lines, and there is no `docs/`.
Why now: each feature adds a paragraph, and the file is loaded in every session. The `condense-claude` skill exists for this.
Options: (a) move the per-file descriptions to `docs/architecture.md` and keep rules, commands and layout in `CLAUDE.md`, (b) leave it. Recommended: (a) at the next feature that touches the architecture text.
Scope: `CLAUDE.md`, new `docs/`.
