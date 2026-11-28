# imetap

A small tray tool for Windows and macOS that makes switching between Chinese and English input painless.

- Global hotkeys switch the input method: `Ctrl+Shift+S` US, `Ctrl+Shift+F` Microsoft Pinyin (Shuangpin on macOS), `Ctrl+Shift+D` Doubao.
- Double-tap `Ctrl` to toggle Chinese/English in the current input method. Triple-tap it to show the HUD without changing anything.
- A small translucent HUD at the bottom of the current window's monitor (red for Chinese, blue for English and the US layout, with a bright stripe on the left whose colour and label name the input method, such as orange "豆" for Doubao. An input method imetap does not know gets a label and colour from its name) shows the new state after every change, and also when you switch to a window whose input method or mode differs (they are per window on Windows, where the HUD then waits until the focus is in a text field, so that passing through windows does not flash it. See `wait_for_text_field`).
- Optionally, a reminder shows the HUD again when you come back after the computer sat idle, and (in `resume-and-switch` mode) when you leave a window you used for a long time without seeing the HUD.
- A tray icon shows the current state at a glance: red "中" for Chinese, blue "E" for English (including the US layout). It can show the logo instead (`tray.enable_lang_indicator`).
- It is event-driven and does not poll.

## Install

Both platforms use the two [uv](https://docs.astral.sh/uv/) scripts in `scripts/`:

```bash
uv run scripts/build.py      # cargo build --release, then lists the binaries (--check runs fmt, clippy and the tests first)
uv run scripts/install.py    # cargo install --path ., stopping and restarting a running imetapd
imetap daemon start
imetap autostart enable
```

On macOS, `scripts/install.py` also builds `~/Applications/imetap.app` (with the imetap icon, and no Dock icon) around the binary and signs it with a self-signed certificate (created in the login keychain on the first run), so the Accessibility permission survives reinstalls. `imetap daemon start` and `imetap autostart` then use the app's executable, so run `imetap autostart enable` again after switching from a plain `cargo install`. An ad-hoc signed binary is identified by its hash and loses the permission on every rebuild. Only once, after the first run (when the certificate is created), remove the old imetap entry under System Settings > Privacy & Security > Accessibility and grant access again. Later runs reuse the certificate and keep the permission. `scripts/install.py --no-install` only rebuilds and re-signs the app from the installed binary, e.g. after a plain `cargo install`.

## Usage

```
imetap daemon run [--debug]        run the background process in this terminal
imetap daemon start | stop         manage the background process
imetap daemon reload               read the configuration file again
imetap switch im us|pinyin|doubao  switch input method (shuangpin on macOS)
imetap switch lang                 toggle Chinese/English
imetap status [-v]                 process, login item and current input method
imetap autostart enable [--debug] | disable | status
imetap paths                       configuration file, debug log and background process
imetap init                        write the default configuration file
imetap config edit [--gui]         open the configuration file in $EDITOR (or the default editor)
```

There are two programs. `imetap` is the command line. `imetapd` is the background process that `imetap daemon start` and the login entry launch (on Windows it has no console window).

`imetap daemon run` runs the background process in the foreground. `imetap daemon run --debug`, `imetapd --debug`, or `IMETAP_DEBUG=1` in its environment, makes it write a debug log to `imetap-debug.log` in the temporary directory (`imetap paths` and `imetap status` show where). `imetap autostart enable --debug` makes the login entry start it that way. The tray menu offers "Run at login", "Open config file" and "Quit".

## Configuration

`config.toml` lives in `$XDG_CONFIG_HOME/imetap/`, or the platform default (`%APPDATA%\imetap` on Windows, `~/Library/Application Support/imetap` on macOS). Every key is optional. `imetap init` writes the defaults with comments, the same as [`config.example.toml`](config.example.toml). Run `imetap daemon reload` after editing.

```toml
# The input methods imetap handles: us, pinyin (Windows), doubao, shuangpin (macOS). One left out gets no hotkey and counts as unknown.
enable = ["us", "pinyin", "doubao", "shuangpin"]

[tap]
enabled = true
gap_ms = 500     # longest time between the two taps
hold_ms = 300    # longest time Ctrl may be held for a press to count as a tap
triple_ms = 300  # wait this long after a double tap for a third (delays the toggle). 0 = off

[hotkeys]        # an empty string disables a hotkey
us = "Ctrl+Shift+S"
chinese = "Ctrl+Shift+F"
doubao = "Ctrl+Shift+D"

[hud]
enabled = true
on_window_switch = true
wait_for_text_field = true  # Windows: on a window switch, show the HUD only once the focus is in a text field
duration_ms = 1500 # how long it stays fully visible
fade_in_ms = 350    # fade-in time, 0 for none
fade_out_ms = 250   # fade-out time, 0 for none
opacity = 0.85

[hud.colors]      # background by mode. Names: red, orange, green, teal, blue, purple, pink, brown, grey, black
chinese = "red"
english = "blue"

[hud.ime_colors]  # the stripe on the left, per input method
us = "grey"
pinyin = "green"
doubao = "orange"
shuangpin = "teal"
other = "purple"

[hud.ime_labels]  # the big label on the stripe, one or two characters
us = "US"
pinyin = "微"
doubao = "豆"
shuangpin = "双"
other = "其"

[reminder]
enabled = false
mode = "resume"  # or "resume-and-switch"
idle_secs = 300  # no key, mouse or focus-window activity for this long counts as idle
dwell_secs = 300 # resume-and-switch: leaving a window used this long without a HUD shows it
```

On macOS the reminder sees changes of the frontmost application, not of windows within one application.

## Notes

- Windows: Microsoft Pinyin has no supported API to set its mode in modern apps, so imetap sends the Windows IME toggle hotkey configured in your settings (read from the registry, `Ctrl+Space` by default). The mode is read from the taskbar's input indicator through UI Automation. Doubao is controlled over its own named pipe.
- macOS: the macOS port is untested. It needs the Accessibility permission, and the double-tap swaps the US source with the last-used Chinese one because there is no mode to read.

## License

MIT
