mod swimmers_tui;

use clap::Parser;
use swimmers::cli::TuiCli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse for --help / --version. The TUI itself takes no other args
    // today; clap will exit early on --help/--version, otherwise we fall
    // through to the existing TUI run loop.
    let _ = TuiCli::parse();
    swimmers_tui::run()
}
