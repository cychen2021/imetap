# Configuration

`config.toml` lives in `$XDG_CONFIG_HOME/imetap/`, or the platform default (`%APPDATA%\imetap` on Windows, `~/Library/Application Support/imetap` on macOS). Every key is optional. `imetap init` writes the defaults with comments, the same as [`config.example.toml`](../config.example.toml). Run `imetap daemon reload` after editing.

```toml
# The input methods imetap handles, among us, pinyin (Windows), doubao and shuangpin (macOS):
# "all_available" for those the system offers, or a list such as ["us", "doubao"]. One left out gets no hotkey and counts as unknown.
enable = "all_available"

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

With `enable = "all_available"`, imetap looks at what the system offers when it starts: on Windows, an English (US) keyboard layout and the Microsoft Pinyin and Doubao profiles turned on in the language settings, and on macOS the input sources in the input menu. After adding an input method to the system, run `imetap daemon reload`.
