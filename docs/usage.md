# Usage

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
