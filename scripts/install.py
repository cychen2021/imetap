#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["typer>=0.12", "rich>=13"]
# ///
"""Installs imetap with cargo. On macOS it also builds ~/Applications/imetap.app and signs it.

An ad-hoc signed binary is identified by its hash, so every `cargo install` looks like a new app
to macOS and loses the Accessibility permission. The app bundle, signed with a fixed certificate
and identifier, keeps the permission across reinstalls and has the imetap icon. `imetap start` and
`imetap autostart` use the bundle's executable when it exists.

Usage: scripts/install.py [--no-install] [--cert NAME]
"""

import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console
from rich.panel import Panel

ROOT = Path(__file__).resolve().parent.parent
IDENTIFIER = "io.github.cychen2021.imetap"
APP = Path.home() / "Applications/imetap.app"
KEYCHAIN = Path.home() / "Library/Keychains/login.keychain-db"
console = Console()

CERT_CONFIG = """\
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = {name}
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
"""


def main(
    no_install: Annotated[
        bool, typer.Option("--no-install", help="Only rebuild and sign the app bundle from the installed binary (macOS).")
    ] = False,
    cert: Annotated[
        str, typer.Option(envvar="IMETAP_CERT", help="Name of the signing certificate (macOS).")
    ] = "imetap Code Signing",
) -> None:
    """Install imetap, and sign it on macOS."""
    mac = sys.platform == "darwin"
    if no_install and not mac:
        console.print("[red]--no-install only makes sense on macOS, where it signs.[/]")
        raise typer.Exit(1)
    first_cert = mac and not has_certificate(cert)
    if first_cert:
        create_certificate(cert)

    was_running = stop_running() if not no_install else False
    if not no_install:
        run(["cargo", "install", "--path", str(ROOT)])

    binary = installed_binary()
    if mac:
        binary = make_bundle(binary)
        sign(APP, cert)
    if was_running or (mac and is_running()):
        run([str(binary), "restart" if mac else "start"], check=False)
    console.print(f"[bold green]Installed[/] {APP if mac else binary}")
    if first_cert:
        console.print(
            Panel(
                "The certificate was just created, so the Accessibility entry granted to the old "
                "ad-hoc signed imetap no longer matches. In System Settings > Privacy & Security > "
                'Accessibility, remove the old imetap entry with "-", then grant access when '
                f'imetap asks (or add {APP} with "+").\n\nYou will not need to do this again: '
                "later runs reuse the same certificate, so the permission survives reinstalls.\n\n"
                "If you enabled autostart before, run `imetap autostart enable` again: the login "
                "entry should point at the app.",
                title="One-time step",
                border_style="yellow",
            )
        )


def run(command: list[str], check: bool = True, **kwargs) -> subprocess.CompletedProcess:
    """Runs a command, echoing it, and exits with its code if it fails and `check` is set."""
    console.print(f"[bold cyan]$[/] {' '.join(command)}")
    result = subprocess.run(command, **kwargs)
    if check and result.returncode != 0:
        console.print(f"[bold red]Failed[/] with exit code {result.returncode}")
        raise typer.Exit(result.returncode)
    return result


def installed_binary() -> Path:
    """The installed `imetap`, from PATH or else from the cargo bin directory."""
    found = shutil.which("imetap")
    exe = "imetap.exe" if sys.platform == "win32" else "imetap"
    path = Path(found) if found else Path.home() / ".cargo/bin" / exe
    if not path.exists():
        console.print(f"[red]imetap is not installed at {path}.[/]")
        raise typer.Exit(1)
    return path


def is_running() -> bool:
    """Whether an `imetap` process is running."""
    if sys.platform == "win32":
        out = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq imetap.exe", "/NH"], capture_output=True, text=True
        ).stdout
        return "imetap.exe" in out
    return subprocess.run(["pgrep", "-qx", "imetap"]).returncode == 0


def stop_running() -> bool:
    """Stops a running imetap, which would lock its executable on Windows. Returns whether it ran."""
    if not is_running():
        return False
    found = shutil.which("imetap")
    if found and sys.platform == "win32":
        run([found, "stop"], check=False)
        return True
    return False


def make_bundle(binary: Path) -> Path:
    """Builds ~/Applications/imetap.app around the installed binary. Returns its executable."""
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    console.print(f"Building {APP}...")
    if APP.exists():
        shutil.rmtree(APP)
    (APP / "Contents/MacOS").mkdir(parents=True)
    (APP / "Contents/Resources").mkdir()
    shutil.copy2(binary, APP / "Contents/MacOS/imetap")
    shutil.copy2(ROOT / "assets/mac/icon.icns", APP / "Contents/Resources/icon.icns")
    info = {
        "CFBundleIdentifier": IDENTIFIER,
        "CFBundleName": "imetap",
        "CFBundleDisplayName": "imetap",
        "CFBundleExecutable": "imetap",
        "CFBundleIconFile": "icon",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "NSHighResolutionCapable": True,
        # A menu bar tool: no Dock icon and no application menu.
        "LSUIElement": True,
    }
    (APP / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    return APP / "Contents/MacOS/imetap"


def has_certificate(name: str) -> bool:
    """Whether the login keychain has the certificate."""
    result = subprocess.run(
        ["security", "find-certificate", "-c", name, str(KEYCHAIN)], capture_output=True
    )
    return result.returncode == 0


def create_certificate(name: str) -> None:
    """Creates the self-signed code signing certificate in the login keychain."""
    console.print(f'Creating the self-signed certificate "{name}" in the login keychain...')
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp)
        (d / "cert.cnf").write_text(CERT_CONFIG.format(name=name))
        # The system LibreSSL writes a PKCS#12 file that `security import` can read. OpenSSL 3
        # would need `-legacy`.
        openssl = "/usr/bin/openssl"
        run(
            [openssl, "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3650"]
            + ["-config", str(d / "cert.cnf"), "-keyout", str(d / "key.pem")]
            + ["-out", str(d / "cert.pem")],
            stderr=subprocess.DEVNULL,
        )
        run(
            [openssl, "pkcs12", "-export", "-inkey", str(d / "key.pem"), "-in", str(d / "cert.pem")]
            + ["-out", str(d / "cert.p12"), "-passout", "pass:imetap"]
        )
        # -T lets codesign use the key without asking each time.
        run(
            ["security", "import", str(d / "cert.p12"), "-k", str(KEYCHAIN), "-P", "imetap"]
            + ["-T", "/usr/bin/codesign"],
            stdout=subprocess.DEVNULL,
        )


def sign(target: Path, cert: str) -> None:
    """Signs the app bundle with the certificate and a fixed identifier."""
    console.print(f"Signing {target}...")
    run(["codesign", "--force", "--sign", cert, "--identifier", IDENTIFIER, str(target)])
    shown = subprocess.run(
        ["codesign", "--display", "--requirements", "-", str(target)],
        capture_output=True,
        text=True,
    )
    for line in (shown.stdout + shown.stderr).splitlines():
        if "designated" in line:
            console.print(line, markup=False)


if __name__ == "__main__":
    typer.run(main)
