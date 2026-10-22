#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 3 && args[1] == "--build-info-to" {
        // Offline metadata only. Do not initialize Tauri or read user credentials.
        let status = slopbar_lib::write_build_info(std::path::Path::new(&args[2]));
        std::process::exit(if status.is_ok() { 0 } else { 1 });
    }
    if args.len() == 3 && args[1] == "--after" {
        // Started by a restart: let the previous instance finish its orderly exit first.
        if let Some(pid) = args[2].to_str().and_then(|s| s.parse().ok()) {
            slopbar_lib::wait_for_exit(pid, std::time::Duration::from_secs(60));
        }
    }
    slopbar_lib::run()
}
