// No console window when an agent host starts it; stdio still works through the host's pipes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Agents' stdio MCP server, a separate executable so app updates (which replace and stop
//! `slopbar.exe`) never kill an agent's server. It starts no app, reads no credentials and
//! contacts no provider: exports go through the running app's local door.
//!
//! With `--statusline` it instead prints one line for Claude Code's status line and exits (see
//! `statusline`); with `--session-hook` it is Claude Code's SessionStart hook (see
//! `session_hook`); with `--turn-hook` its UserPromptSubmit and Stop hook for graceful rotation (see
//! `turn_hook`); `--pick` and `--release` serve the `slopbar-run` launcher (see `launcher`);
//! `--call` runs one tool and prints its result, which is how another computer reaches this one
//! over SSH, and `--pair-receive` takes a pairing key on standard input (see `pairing`).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--statusline") => slopbar_lib::statusline(),
        Some("--session-hook") => slopbar_lib::session_hook(),
        Some("--turn-hook") => slopbar_lib::turn_hook(),
        Some("--pick" | "--release") => std::process::exit(slopbar_lib::launcher_helper(&args)),
        Some("--call" | "--call-b64") => std::process::exit(slopbar_lib::call_cli(&args)),
        Some("--pair-receive") => std::process::exit(slopbar_lib::pair_receive()),
        _ => std::process::exit(if slopbar_lib::serve_mcp().is_ok() { 0 } else { 1 }),
    }
}
