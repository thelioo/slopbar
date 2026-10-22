#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 3 && args[1] == "--build-info-to" {
        // Offline metadata only. Do not initialize Tauri or read user credentials.
        let status = slopbar_lib::write_build_info(std::path::Path::new(&args[2]));
        std::process::exit(if status.is_ok() { 0 } else { 1 });
    }
    // Started by a restart: the previous instance finishes its orderly exit first.
    let after = (args.len() == 3 && args[1] == "--after").then(|| args[2].to_str().and_then(|s| s.parse().ok())).flatten();
    // Before anything reads app data: a second instance hands over and exits.
    if !slopbar_lib::claim_instance(after) {
        return;
    }
    slopbar_lib::run()
}
