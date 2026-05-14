// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if codexl_lib::run_cli_middleware_if_requested() {
        return;
    }

    codexl_lib::run()
}
