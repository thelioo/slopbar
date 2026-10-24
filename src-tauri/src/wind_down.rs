//! Graceful rotation ("wind-down"): when the signed-in Claude account reaches its own rotation
//! percentage (`Settings::rotate_at`), the switch waits until the Claude Code sessions running
//! on that account have finished their current turn and saved their state, instead of pulling
//! the account from under them.
//!
//! Sessions report their turns through `slopbar-mcp --turn-hook` (Claude Code's UserPromptSubmit
//! and Stop hooks, see `turn_hook`). While an account winds down:
//! - a prompt submitted on it gets context saying the account is rotating: finish this task, don't
//!   start new work;
//! - the first time one of its sessions ends a turn, SlopBar blocks that stop once with the
//!   wrap-up instruction (update durable memory and each project's canonical state on disk). The
//!   session's next stop after that block is its wrap-up finishing: it is done.
//!
//! The switch happens when no session on the account is still busy or wrapping up, when the
//! grace period ends, or at once when usage reaches the hard limit (`hard_limit`). Evidence is
//! SlopBar's own: a session counts as done only after the stop SlopBar blocked was followed by
//! another stop, never because of `stop_hook_active` (any Stop hook sets that).
//!
//! State is in memory only. After a restart the next refresh starts a new wind-down, and a session
//! that already wrapped up is asked once more at its next stop, which costs one short turn.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

/// At most this many sessions' turns are remembered.
const MAX_TURNS: usize = 256;
/// A turn not heard from for this long no longer holds a switch up (a crashed or closed session).
const STALE_TURN: Duration = Duration::hours(2);

/// The usage at which a graceful switch stops waiting: a few points past the threshold, never
/// the limit itself.
pub fn hard_limit(threshold: u8) -> f64 {
    (threshold as f64 + 5.0).min(98.0)
}

#[derive(Debug, Clone)]
struct Turn {
    account: String,
    busy: bool,
    at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Working on a turn that started before or during the wind-down.
    Busy,
    /// SlopBar blocked its stop with the wrap-up instruction; it is saving its state.
    Wrapping,
    /// Wrapped up (its next stop after the block came).
    Done,
}

#[derive(Debug, Clone)]
pub struct Active {
    /// The account winding down; the account to switch to is picked when the switch happens.
    pub from: String,
    pub deadline: DateTime<Utc>,
    phases: HashMap<String, (Phase, DateTime<Utc>)>,
}

impl Active {
    /// Sessions still busy or wrapping up (heard from recently).
    pub fn waiting_on(&self, now: DateTime<Utc>) -> usize {
        self.phases.values().filter(|(p, at)| *p != Phase::Done && now - *at < STALE_TURN).count()
    }
}

/// What the hook prints for a turn event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Nothing: the session carries on as usual.
    None,
    /// Context added to the prompt (UserPromptSubmit).
    Context(String),
    /// The stop is blocked once: the session goes on with this instruction (Stop).
    Block(String),
}

/// Claude Code's turn events SlopBar listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Prompt,
    Stop,
}

#[derive(Debug, Clone, Default)]
pub struct Tracker {
    turns: HashMap<String, Turn>,
    pub active: Option<Active>,
}

pub const PROMPT_CONTEXT: &str = "SlopBar: the Claude account this session runs on reached its usage threshold and will be rotated as soon as running sessions finish their current turn. Finish this task without starting new work; you will be asked to save your state when you stop.";

pub const WRAP_UP: &str = "SlopBar: the Claude account this session runs on reached its usage threshold and is being rotated. Before ending your turn, wind down: update durable memory and the canonical state on disk (handoff, checkpoint or status files, following each project's own conventions) for every project you worked in during this session, so a fresh session on another account can continue. Commit only where the project's rules say to. Do not start new work. Then end your turn.";

impl Tracker {
    /// Records a turn event from session `session` on `account`, and what the hook should print.
    pub fn on_event(&mut self, session: &str, account: &str, event: Event, now: DateTime<Utc>) -> Reply {
        self.remember(session, account, event == Event::Prompt, now);
        let Some(active) = self.active.as_mut().filter(|a| a.from == account) else { return Reply::None };
        let phase = active.phases.get(session).map(|(p, _)| *p);
        match event {
            Event::Prompt => {
                // New work in a session that had wrapped up (the user asked for more): it is busy
                // again and wraps up again at its next stop.
                active.phases.insert(session.to_string(), (Phase::Busy, now));
                Reply::Context(PROMPT_CONTEXT.into())
            }
            Event::Stop => match phase {
                Some(Phase::Wrapping) => {
                    active.phases.insert(session.to_string(), (Phase::Done, now));
                    Reply::None
                }
                Some(Phase::Done) => Reply::None,
                // Busy, or a turn that began before SlopBar heard of it: wrap up once.
                Some(Phase::Busy) | None => {
                    active.phases.insert(session.to_string(), (Phase::Wrapping, now));
                    Reply::Block(WRAP_UP.into())
                }
            },
        }
    }

    fn remember(&mut self, session: &str, account: &str, busy: bool, now: DateTime<Utc>) {
        if self.turns.len() >= MAX_TURNS && !self.turns.contains_key(session) {
            if let Some(oldest) = self.turns.iter().min_by_key(|(_, t)| t.at).map(|(k, _)| k.clone()) {
                self.turns.remove(&oldest);
            }
        }
        self.turns.insert(session.to_string(), Turn { account: account.to_string(), busy, at: now });
    }

    /// Starts winding `from` down. Sessions in the middle of a turn on `from` are
    /// waited on; idle ones are not (they wrap up if they start a turn before the switch).
    pub fn start(&mut self, from: &str, grace: Duration, now: DateTime<Utc>) {
        let phases = self.turns.iter()
            .filter(|(_, t)| t.account == from && t.busy && now - t.at < STALE_TURN)
            .map(|(s, t)| (s.clone(), (Phase::Busy, t.at)))
            .collect();
        self.active = Some(Active { from: from.into(), deadline: now + grace, phases });
    }

    /// Whether the switch may happen now: nobody left to wait for, or the grace period is over.
    pub fn ready(&self, now: DateTime<Utc>) -> bool {
        self.active.as_ref().is_some_and(|a| now >= a.deadline || a.waiting_on(now) == 0)
    }

    pub fn finish(&mut self) -> Option<Active> {
        self.active.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(min: i64) -> DateTime<Utc> {
        "2026-10-22T10:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::minutes(min)
    }

    #[test]
    fn passes_through_when_nothing_winds_down() {
        let mut t = Tracker::default();
        assert_eq!(t.on_event("s1", "main", Event::Prompt, at(0)), Reply::None);
        assert_eq!(t.on_event("s1", "main", Event::Stop, at(1)), Reply::None);
        assert!(!t.ready(at(1)), "nothing to switch");
    }

    #[test]
    fn busy_session_wraps_up_once_then_the_switch_is_ready() {
        let mut t = Tracker::default();
        t.on_event("s1", "main", Event::Prompt, at(0));
        t.on_event("idle", "main", Event::Stop, at(0));
        t.on_event("other", "spare", Event::Prompt, at(0));
        t.start("main", Duration::minutes(20), at(1));
        assert!(!t.ready(at(2)), "s1 is mid-turn");
        assert_eq!(t.on_event("s1", "main", Event::Stop, at(3)), Reply::Block(WRAP_UP.into()));
        assert!(!t.ready(at(3)), "s1 is saving its state");
        assert_eq!(t.on_event("s1", "main", Event::Stop, at(4)), Reply::None);
        assert!(t.ready(at(4)));
        // Sessions on another account are never touched.
        assert_eq!(t.on_event("other", "spare", Event::Stop, at(4)), Reply::None);
    }

    #[test]
    fn idle_at_start_means_no_wait() {
        let mut t = Tracker::default();
        t.on_event("s1", "main", Event::Prompt, at(0));
        t.on_event("s1", "main", Event::Stop, at(1));
        t.start("main", Duration::minutes(20), at(2));
        assert!(t.ready(at(2)));
    }

    #[test]
    fn new_prompt_during_wind_down_is_told_and_waited_on() {
        let mut t = Tracker::default();
        t.start("main", Duration::minutes(20), at(0));
        assert_eq!(t.on_event("s2", "main", Event::Prompt, at(1)), Reply::Context(PROMPT_CONTEXT.into()));
        assert!(!t.ready(at(1)));
        assert!(matches!(t.on_event("s2", "main", Event::Stop, at(2)), Reply::Block(_)));
        t.on_event("s2", "main", Event::Stop, at(3));
        assert!(t.ready(at(3)));
        // More work after wrapping up: wrap up again.
        t.on_event("s2", "main", Event::Prompt, at(4));
        assert!(!t.ready(at(4)));
    }

    #[test]
    fn unknown_turn_stopping_is_asked_to_wrap_up() {
        let mut t = Tracker::default();
        t.start("main", Duration::minutes(20), at(0));
        assert!(matches!(t.on_event("s3", "main", Event::Stop, at(1)), Reply::Block(_)));
        assert!(!t.ready(at(1)));
    }

    #[test]
    fn deadline_and_stale_sessions_never_hold_it_forever() {
        let mut t = Tracker::default();
        t.on_event("s1", "main", Event::Prompt, at(0));
        t.start("main", Duration::minutes(20), at(1));
        assert!(!t.ready(at(20)));
        assert!(t.ready(at(21)), "grace period over");
        let mut t = Tracker::default();
        t.on_event("s1", "main", Event::Prompt, at(0));
        t.start("main", Duration::hours(5), at(1));
        assert!(t.ready(at(121)), "not heard from in two hours");
    }

    #[test]
    fn hard_limit_stays_below_the_limit() {
        assert_eq!(hard_limit(90), 95.0);
        assert_eq!(hard_limit(95), 98.0);
    }
}
