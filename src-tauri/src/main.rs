// Keeps a console window from appearing alongside the app on Windows release
// builds. No effect on macOS or Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    guac_lib::run()
}
