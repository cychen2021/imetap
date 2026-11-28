//! `imetap`, the command line: manages the resident process `imetapd`, switches input methods and
//! edits the configuration. It is a console program on every platform, so its output works in pipes.

use anyhow::Result;

fn main() -> Result<()> {
    imetap::cli::main()
}
