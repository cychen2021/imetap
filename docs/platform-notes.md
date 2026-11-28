# Platform notes

- Windows: Microsoft Pinyin has no supported API to set its mode in modern apps, so imetap sends the Windows IME toggle hotkey configured in your settings (read from the registry, `Ctrl+Space` by default). The mode is read from the taskbar's input indicator through UI Automation. Doubao is controlled over its own named pipe.
- macOS: the macOS port is untested. It needs the Accessibility permission, and the double-tap swaps the US source with the last-used Chinese one because there is no mode to read.
