# Install

Both platforms use the two [uv](https://docs.astral.sh/uv/) scripts in `scripts/`:

```bash
uv run scripts/build.py      # cargo build --release, then lists the binaries (--check runs fmt, clippy and the tests first)
uv run scripts/install.py    # cargo install --path ., stopping and restarting a running imetapd
imetap daemon start
imetap autostart enable
```

On macOS, `scripts/install.py` also builds `~/Applications/imetap.app` (with the imetap icon, and no Dock icon) around the binary and signs it with a self-signed certificate (created in the login keychain on the first run), so the Accessibility permission survives reinstalls. `imetap daemon start` and `imetap autostart` then use the app's executable, so run `imetap autostart enable` again after switching from a plain `cargo install`. An ad-hoc signed binary is identified by its hash and loses the permission on every rebuild. Only once, after the first run (when the certificate is created), remove the old imetap entry under System Settings > Privacy & Security > Accessibility and grant access again. Later runs reuse the certificate and keep the permission. `scripts/install.py --no-install` only rebuilds and re-signs the app from the installed binary, e.g. after a plain `cargo install`.
