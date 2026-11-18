// No console window in release builds (tray app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod toast;
mod win;

fn main() {
    if std::env::args().any(|a| a == "--facts") {
        app::print_facts();
        return;
    }
    app::run();
}
