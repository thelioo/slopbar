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
mod unexpected;
mod health;
mod pins;
mod bulk;
mod auto_monitor;
mod calendar;
mod digest;
mod statusline;
mod session_hook;
mod leases;
mod launcher;
pub use session_hook::run as session_hook;
pub use launcher::helper as launcher_helper;
pub use statusline::run as statusline;
pub use diagnostics::write_build_info;
mod door;
mod instance;
pub use instance::claim as claim_instance;
mod mcp;
pub use mcp::serve_stdio as serve_mcp;
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
