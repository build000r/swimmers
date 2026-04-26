mod swimmers_tui;

use clap::Parser;
use swimmers::cli::TuiCli;

fn main() {
    // Parse for --help / --version. The TUI itself takes no other args
    // today; clap will exit early on --help/--version, otherwise we fall
    // through to the existing TUI run loop.
    let _ = TuiCli::parse();
    if let Err(err) = swimmers_tui::run() {
        // Use Display formatting so users see a readable message instead of
        // the Debug noise Rust prints when `main` returns `Err`.
        eprintln!("swimmers-tui: {err}");
        std::process::exit(1);
    }
}
