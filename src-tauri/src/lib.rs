// The MCP settings schema is one large json! literal; the default macro recursion limit (128) is too low.
#![recursion_limit = "256"]
#![cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod accounts;
mod login;
mod balancer;
mod model;
mod settings;
mod sources;
mod providers;
mod monitoring;
mod private_store;
mod diagnostics;
mod subscriptions;
mod headroom;
mod plan_alerts;
mod advisor;
mod burn;
mod reset_credits;
mod ledger;
mod backup;
mod phone;
mod codex_local;
mod active_log;
mod usage_index;
mod csv_export;
mod presence;
mod rclone;
mod roster;
mod unexpected;
mod health;
mod pins;
mod bulk;
mod signin;
mod codex_signin;
pub use signin::launch_from_env as signin_launch;
mod auto_monitor;
mod calendar;
mod digest;
mod statusline;
mod session_hook;
mod turn_hook;
mod wind_down;
mod leases;
mod launcher;
mod i18n;
mod i18n_format;
mod alerts;
mod widget;
#[cfg(windows)]
mod widget_draw;
#[cfg(all(windows, feature = "desktop"))]
mod widget_win;
pub use session_hook::run as session_hook;
pub use turn_hook::run as turn_hook;
pub use launcher::helper as launcher_helper;
pub use statusline::run as statusline;
pub use diagnostics::write_build_info;
mod door;
mod desktop;
mod pairing;
mod instance;
pub use instance::claim as claim_instance;
mod mcp;
pub use mcp::serve_stdio as serve_mcp;
pub use mcp::{call_cli, pair_receive};
#[cfg(feature = "desktop")]
mod dock;
#[cfg(feature = "desktop")]
mod inspect;
#[cfg(feature = "desktop")]
mod migrate;
#[cfg(feature = "desktop")]
mod notify;
#[cfg(feature = "desktop")]
mod panel;
#[cfg(feature = "desktop")]
mod updates;
#[cfg(feature = "desktop")]
mod app;
#[cfg(feature = "desktop")]
pub use app::run;
#[cfg(feature = "desktop")]
pub(crate) use app::AppState;

#[cfg(test)]
mod test_support;
