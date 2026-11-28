#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["typer>=0.12", "rich>=13"]
# ///
"""Builds imetap and shows the resulting binaries.

Usage: scripts/build.py [--debug] [--target TRIPLE] [--check]
"""

import subprocess
import sys
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console
from rich.table import Table

ROOT = Path(__file__).resolve().parent.parent
BINARIES = ("imetap", "imetapd")
console = Console()


def main(
    debug: Annotated[bool, typer.Option("--debug", help="Build the debug profile.")] = False,
    target: Annotated[str | None, typer.Option(help="Build for this target triple.")] = None,
    check: Annotated[
        bool, typer.Option("--check", help="Run fmt, clippy and the tests first.")
    ] = False,
) -> None:
    """Build imetap and print where the binaries are."""
    if check:
        for step in (
            ["cargo", "fmt", "--check"],
            ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
            ["cargo", "test"],
        ):
            run(step)
    command = ["cargo", "build"]
    if not debug:
        command.append("--release")
    if target:
        command += ["--target", target]
    run(command)

    profile = "debug" if debug else "release"
    directory = ROOT / "target" / target / profile if target else ROOT / "target" / profile
    windows = "windows" in target if target else sys.platform == "win32"
    suffix = ".exe" if windows else ""
    table = Table("Binary", "Size", "Path")
    for name in BINARIES:
        path = directory / f"{name}{suffix}"
        if path.exists():
            table.add_row(name, f"{path.stat().st_size / 1024:.0f} KiB", str(path))
    console.print(table)


def run(command: list[str]) -> None:
    """Runs a command in the repository root and exits with its code if it fails."""
    console.print(f"[bold cyan]$[/] {' '.join(command)}")
    result = subprocess.run(command, cwd=ROOT)
    if result.returncode != 0:
        console.print(f"[bold red]Failed[/] with exit code {result.returncode}")
        raise typer.Exit(result.returncode)


if __name__ == "__main__":
    typer.run(main)
