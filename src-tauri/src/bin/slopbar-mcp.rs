// No console window when an agent host starts it; stdio still works through the host's pipes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Agents' stdio MCP server, a separate executable so app updates (which replace and stop
//! `slopbar.exe`) never kill an agent's server. It starts no app, reads no credentials and
//! contacts no provider: exports go through the running app's local door.
//!
//! With `--statusline` it instead prints one line for Claude Code's status line and exits (see
//! `statusline`).
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--statusline") {
        slopbar_lib::statusline();
        return;
    }
    std::process::exit(if slopbar_lib::serve_mcp().is_ok() { 0 } else { 1 });
}
