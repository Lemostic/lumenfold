// Hide the console window that Windows would otherwise attach to the release binary.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lumenfold_desktop_lib::run()
}