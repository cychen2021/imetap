# imetap

A small tray tool for Windows and macOS that makes switching between Chinese and English input painless.

- Global hotkeys switch the input method: `Ctrl+Shift+S` US, `Ctrl+Shift+F` Microsoft Pinyin (Shuangpin on macOS), `Ctrl+Shift+D` Doubao.
- Double-tap `Ctrl` to toggle Chinese/English in the current input method.
- A small translucent HUD at the bottom of the current window's monitor shows the new state after every change, and also when you switch to a window whose input method or mode differs (they are per window on Windows).
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

`imetap run` runs in the foreground. The tray menu offers "Run at login", "Open config file" and "Quit".

## Configuration

`config.toml` lives in `$XDG_CONFIG_HOME/imetap/`, or the platform default (`%APPDATA%\imetap` on Windows, `~/Library/Application Support/imetap` on macOS). Every key is optional. `imetap config init` writes the defaults with comments; restart with `imetap restart` after editing.

```toml
[tap]
enabled = true
gap_ms = 500     # longest time between the two taps
hold_ms = 300    # longest time Ctrl may be held for a press to count as a tap

[hotkeys]        # an empty string disables a hotkey
us = "Ctrl+Shift+S"
chinese = "Ctrl+Shift+F"
doubao = "Ctrl+Shift+D"

[hud]
enabled = true
on_window_switch = true
duration_ms = 900
opacity = 0.85
```

## Notes

- Windows: Microsoft Pinyin has no supported API to set its mode in modern apps, so imetap sends the Windows IME toggle hotkey configured in your settings (read from the registry, `Ctrl+Space` by default). The mode is read from the taskbar's input indicator through UI Automation. Doubao is controlled over its own named pipe.
- macOS: the macOS port is untested; it needs the Accessibility permission, and the double-tap swaps the US source with the last-used Chinese one because there is no mode to read.

## License

MIT
