// No console window next to the tray app on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    musichub_sync_lib::run()
}
