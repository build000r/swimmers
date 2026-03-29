mod swimmers_tui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    swimmers_tui::run()
}
