# imetap

A small tray tool for Windows and macOS that makes switching between Chinese and English input painless.

- Global hotkeys switch the input method: `Ctrl+Shift+S` US, `Ctrl+Shift+F` Microsoft Pinyin (Shuangpin on macOS), `Ctrl+Shift+D` Doubao.
- Double-tap `Ctrl` to toggle Chinese/English in the current input method. Triple-tap it to show the HUD without changing anything.
- A small translucent HUD at the bottom of the current window's monitor (red for Chinese, blue for English and the US layout, with a bright stripe on the left whose colour and label name the input method, such as orange "豆" for Doubao. An input method imetap does not know gets a label and colour from its name) shows the new state after every change, and also when you switch to a window whose input method or mode differs (they are per window on Windows, where the HUD then waits until the focus is in a text field, so that passing through windows does not flash it. See `wait_for_text_field`).
- Optionally, a reminder shows the HUD again when you come back after the computer sat idle, and (in `resume-and-switch` mode) when you leave a window you used for a long time without seeing the HUD.
- A tray icon shows the current state at a glance: red "中" for Chinese, blue "E" for English (including the US layout). It can show the logo instead (`tray.enable_lang_indicator`).
- It is event-driven and does not poll.

## Documentation

The full documentation is in [`docs/`](docs/index.md):

- [Install](docs/install.md)
- [Usage](docs/usage.md)
- [Configuration](docs/configuration.md)
- [Platform notes](docs/platform-notes.md)

## Quick start

```bash
uv run scripts/install.py
imetap daemon start
imetap autostart enable
imetap --help
```

## License

MIT
