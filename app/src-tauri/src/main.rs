// Keeps a console window from opening alongside the launcher on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    voxear_launcher_app::run()
}
