#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    clawpal::path_fix::ensure_tool_paths();
    clawpal::run();
}
