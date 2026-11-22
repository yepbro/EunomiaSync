// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// No `eprintln!`/`eprint!` in production code (T-255, D-032/D-129: a
// GUI release build has no stderr, and the `eprintln!` panic under
// `panic = "abort"` would abort the process) — the same gate as lib.rs.
#![deny(clippy::print_stderr)]

fn main() {
    eunomia_sync_lib::run()
}
