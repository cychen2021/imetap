# imetap

A small tray tool for Windows and macOS that makes switching between Chinese and English input painless.

- Global hotkeys switch the input method: `Ctrl+Shift+S` US, `Ctrl+Shift+F` Microsoft Pinyin (Shuangpin on macOS), `Ctrl+Shift+D` Doubao.
- Double-tap `Ctrl` to toggle Chinese/English in the current input method; triple-tap it to show the HUD without changing anything.
- A small translucent HUD at the bottom of the current window's monitor (red for Chinese, blue for English and the US layout) shows the new state after every change, and also when you switch to a window whose input method or mode differs (they are per window on Windows).
- Optionally, a reminder shows the HUD again when you come back after the computer sat idle, and (in `resume-and-switch` mode) when you leave a window you used for a long time without seeing the HUD.
- A tray icon shows the current state at a glance: red for Chinese, blue for English, grey for the US layout.
- It is event-driven and does not poll.

## Install

```bash
cargo install --path .
imetap start
imetap autostart enable
```

## Usage

```
imetap start | stop | restart      manage the background process
imetap status [-v]                 process, login item and current input method
imetap set us|pinyin|doubao        switch input method (shuangpin on macOS)
imetap toggle                      toggle Chinese/English
imetap autostart enable|disable|status
imetap config path|init|edit|check
```

On Windows, `imetap.exe` is a GUI-subsystem program, so its output does not reach pipes or scripts. `imetapctl` takes the same commands (except that it starts nothing by itself) as a console program with normal output; it launches and registers the `imetap` next to it. On macOS it is only a stub; use `imetap` there.

`imetap run` runs in the foreground. The tray menu offers "Run at login", "Open config file" and "Quit".

## Configuration

`config.toml` lives in `$XDG_CONFIG_HOME/imetap/`, or the platform default (`%APPDATA%\imetap` on Windows, `~/Library/Application Support/imetap` on macOS). Every key is optional. `imetap config init` writes the defaults with comments, the same as [`config.example.toml`](config.example.toml); restart with `imetap restart` after editing.

```toml
[tap]
enabled = true
gap_ms = 500     # longest time between the two taps
hold_ms = 300    # longest time Ctrl may be held for a press to count as a tap
triple_ms = 300  # wait this long after a double tap for a third (delays the toggle); 0 = off

[hotkeys]        # an empty string disables a hotkey
us = "Ctrl+Shift+S"
chinese = "Ctrl+Shift+F"
doubao = "Ctrl+Shift+D"

[hud]
enabled = true
on_window_switch = true
duration_ms = 900
opacity = 0.85

[reminder]
enabled = false
mode = "resume"  # or "resume-and-switch"
idle_secs = 300  # no key, mouse or focus-window activity for this long counts as idle
dwell_secs = 300 # resume-and-switch: leaving a window used this long without a HUD shows it
```

On macOS the reminder sees changes of the frontmost application, not of windows within one application.

## Notes

- Windows: Microsoft Pinyin has no supported API to set its mode in modern apps, so imetap sends the Windows IME toggle hotkey configured in your settings (read from the registry, `Ctrl+Space` by default). The mode is read from the taskbar's input indicator through UI Automation. Doubao is controlled over its own named pipe.
- macOS: the macOS port is untested; it needs the Accessibility permission, and the double-tap swaps the US source with the last-used Chinese one because there is no mode to read.

## License

MIT
