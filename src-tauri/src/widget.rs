//! The native taskbar widget's model: what it shows, where, in which colors, and what its
//! tooltips say. Pure (no Win32), so the drawing math is tested on its own; `widget_win` draws
//! it with Direct2D. It mirrors the web widget (src/main.js `renderCompact`, src/styles.css
//! `#widget`), which stays as the fallback for one release (setting `native_widget`).
//!
//! Every length here is in DIPs (CSS pixels); the window scales them by the monitor's DPI.

use crate::accounts::Provider;
use crate::model::{AccountUsage, Snapshot};

/// The widget box sits 4 px inside the taskbar's height, like the tray buttons next to it.
pub const INSET_Y: f32 = 4.0;
/// The box's border (transparent until hovered); part of the measured width.
pub const BORDER: f32 = 1.0;
/// Horizontal padding around the rings, and around a line of text.
pub const PAD_CHIPS: f32 = 6.0;
pub const PAD_TEXT: f32 = 8.0;
/// One provider: its usage ring with the mark inside.
pub const RING_BOX: f32 = 24.0;
pub const GAP: f32 = 4.0;
/// The ring of the web widget: r=6 and stroke 1.6 in a 15-unit view box drawn 24 px wide.
pub const RING_RADIUS: f32 = 6.0 * RING_BOX / 15.0;
pub const RING_STROKE: f32 = 1.6 * RING_BOX / 15.0;
pub const FONT_SIZE: f32 = 12.0;
pub const UNKNOWN_FONT_SIZE: f32 = 11.0;
/// The health warning dot, in the box's top-right corner.
pub const DOT_SIZE: f32 = 6.0;
/// The overlap badge's diameter; drawn with a knock-out edge so it reads over the ring.
pub const BADGE_SIZE: f32 = 8.0;
pub const CORNER: f32 = 4.0;

/// An RGBA color, components 0..=1 (straight alpha).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

const fn rgb(hex: u32) -> Color {
    rgba(hex, 1.0)
}

const fn rgba(hex: u32, a: f32) -> Color {
    Color {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn of_taskbar(light: bool) -> Theme {
        if light { Theme::Light } else { Theme::Dark }
    }
}

/// The CSS tokens the web widget uses (styles.css `:root` and `[data-theme="light"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub text: Color,
    pub text_2: Color,
    pub track: Color,
    pub ok: Color,
    pub warn: Color,
    pub bad: Color,
    pub hover: Color,
    pub hover_border_top: Color,
    pub hover_border: Color,
    pub hover_border_bottom: Color,
    pub press: Color,
    /// Around small badges drawn over a ring: the taskbar's own tone.
    pub badge_edge: Color,
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            text: rgb(0xffffff),
            text_2: rgba(0xffffff, 0.786),
            track: rgba(0xffffff, 0.1),
            ok: rgb(0x6ccb5f),
            warn: rgb(0xfce100),
            bad: rgb(0xff99a4),
            hover: rgba(0xffffff, 0.0605),
            hover_border_top: rgba(0xffffff, 0.1),
            hover_border: rgba(0xffffff, 0.0605),
            hover_border_bottom: rgba(0xffffff, 0.0605),
            press: rgba(0xffffff, 0.0419),
            badge_edge: rgb(0x1c1c1c),
        },
        Theme::Light => Palette {
            text: rgba(0x000000, 0.896),
            text_2: rgba(0x000000, 0.62),
            track: rgba(0x000000, 0.1),
            ok: rgb(0x0f7b0f),
            warn: rgb(0x9d5d00),
            bad: rgb(0xc42b1c),
            hover: rgba(0xffffff, 0.5),
            hover_border_top: rgba(0x000000, 0.02),
            hover_border: rgba(0x000000, 0.02),
            hover_border_bottom: rgba(0x000000, 0.06),
            press: rgba(0xffffff, 0.3),
            badge_edge: rgb(0xeeeeee),
        },
    }
}

/// The ring's color for a share used, as `color()` in main.js.
pub fn usage_color(p: &Palette, percent: f64) -> Color {
    if percent >= 90.0 {
        p.bad
    } else if percent >= 70.0 {
        p.warn
    } else {
        p.ok
    }
}

/// The used arc: it starts at 12 o'clock and runs clockwise, in degrees. The web ring is an SVG
/// circle rotated -90° with a dash as long as the share used, capped at 100%.
pub fn arc_degrees(percent: f64) -> (f32, f32) {
    let share = if percent.is_finite() { percent.clamp(0.0, 100.0) } else { 0.0 };
    (-90.0, (share / 100.0 * 360.0) as f32)
}

/// A point on the ring at `degrees` (0 = 3 o'clock, clockwise, y down), around `center`.
pub fn point_on_ring(center: (f32, f32), radius: f32, degrees: f32) -> (f32, f32) {
    let r = degrees.to_radians();
    (center.0 + radius * r.cos(), center.1 + radius * r.sin())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A provider's ring; `percent` is None when its usage is unknown (stale or failing): the web
    /// widget then shows a "?" by the mark instead of the ring.
    Chip { provider: Provider, percent: Option<f64>, tip: String },
    /// A line of text in place of the rings: loading, no accounts, or a short notice (dim for
    /// the first two, like `.dim`).
    Text { text: String, dim: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub items: Vec<Item>,
    /// Settings > Health has problems: a warning dot, and a line in the tooltip.
    pub health_problems: usize,
    /// The window's accessible name (`aria-label` on the web widget).
    pub label: String,
    /// What each item's tooltip says below its headline (`account_detail`), one per item; empty
    /// for text items.
    pub details: Vec<String>,
    /// Per item, whether its account is in use on several machines at once (`overlap`): an amber
    /// badge on the ring. Missing entries are false.
    pub overlap: Vec<bool>,
}

/// Strings the view needs, already translated (see `crate::i18n`).
pub trait Texts {
    fn t(&self, key: &str, vars: &[(&str, &str)]) -> String;
    /// The language code, for dates and durations (`i18n_format`).
    fn lang(&self) -> &str {
        "en"
    }
}

pub struct Lang(pub String);

impl Texts for Lang {
    fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        crate::i18n::t(&self.0, key, vars)
    }
    fn lang(&self) -> &str {
        &self.0
    }
}

pub fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

/// The window closest to its limit is what matters at a glance (`peak` in main.js).
pub fn peak(a: &AccountUsage) -> Option<f64> {
    a.windows.iter().map(|w| w.used_percent).fold(None, |m: Option<f64>, p| Some(m.map_or(p, |m| m.max(p))))
}

/// A ring's tooltip: "Claude 42%", or "Claude — usage unknown".
pub fn chip_tip(texts: &dyn Texts, provider: Provider, percent: Option<f64>) -> String {
    match percent {
        Some(p) => format!("{} {}%", provider_name(provider), p.round() as i64),
        None => format!("{} — {}", provider_name(provider), texts.t("usageUnknown", &[])),
    }
}

/// Everything the flyout's full card says about an account, for the ring's tooltip below its
/// headline (as `accountTip` in accounttext.js, without the plan details the widget doesn't
/// load): who it is, where it is in use, each window with its reset, the forecast, how current
/// the reading is, monitoring, credits and resets held, and a lease. Groups are separated by a
/// blank line; empty fields are left out.
pub fn account_detail(texts: &dyn Texts, a: &AccountUsage, now: chrono::DateTime<chrono::Utc>) -> String {
    use crate::i18n_format as f;
    let lang = texts.lang();
    let provider = provider_name(a.provider);
    let name = a.alias.clone().or_else(|| a.email.clone()).unwrap_or_else(|| texts.t("unknownAccount", &[]));
    let desktop_app = texts.t("desktopApp", &[]);
    let place = |w: &String| if w == crate::desktop_apps::PLACE { desktop_app.clone() } else { w.clone() };
    let machine = |r: &crate::model::RemoteSeat| {
        if r.where_.is_empty() || (r.where_.len() == 1 && r.where_[0] == "Windows") { r.machine.clone() } else { format!("{} ({})", r.machine, r.where_.iter().map(place).collect::<Vec<_>>().join(", ")) }
    };
    let live: Vec<String> = a.remote.iter().filter(|r| r.live).map(machine).collect();
    let mut here = if a.active { a.sources.clone() } else { vec![] };
    if a.desktop { here.push(desktop_app.clone()); }
    let in_use = if !here.is_empty() {
        format!("{}: {}", texts.t("inUse", &[]), [here, live.clone()].concat().join(", "))
    } else if !live.is_empty() {
        texts.t("inUseOn", &[("list", &live.join(", "))])
    } else {
        String::new()
    };
    let ago = |at: chrono::DateTime<chrono::Utc>| f::duration(lang, ((now - at).num_minutes()).max(1));
    let mut who = vec![
        match (&a.alias, &a.email) { (Some(_), Some(email)) => format!("{name} ({email})"), _ => name.clone() },
        [Some(provider.to_string()), a.plan.clone(), a.org.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
        in_use,
    ];
    if let Some(d) = &a.seats {
        let unknown = texts.t("seatsUnknown", &[]);
        who.push(texts.t("seatsDrift", &[("cli", d.cli.as_deref().unwrap_or("-")), ("desktop", d.desktop.as_deref().unwrap_or(&unknown))]));
    }
    if a.overlap.len() > 1 {
        let n = a.overlap.len().to_string();
        who.push(texts.t("overlapOn", &[("n", &n), ("list", &a.overlap.join(", "))]));
        who.push(texts.t("overlapWhy", &[("n", &n)]));
    }
    who.extend(a.remote.iter().filter(|r| !r.live).map(|r| texts.t("seenOn", &[("machine", &machine(r)), ("t", &ago(r.seen_at))])));
    if a.used_outside { who.push(texts.t("usedOutside", &[])); }
    if let Some(purpose) = &a.leased { who.push(texts.t("leased", &[("purpose", purpose)])); }

    let used = texts.t("used", &[]);
    let mut usage: Vec<String> = a.windows.iter().map(|w| {
        let line = format!("{}: {}% {used}", f::window_label(lang, &w.kind, w.window_seconds), f::js_round(w.used_percent) as i64);
        if f::shows_reset(&a.windows, w) { format!("{line} · {}", f::window_reset_text(lang, w, a.stale, a.fetched_at, now)) } else { line }
    }).collect();
    if let (Some(at), false) = (a.runs_out_at, a.stale) { usage.push(texts.t("runsOut", &[("at", &f::day_time(lang, at))])); }

    let mut state = vec![];
    if let Some(at) = a.fetched_at {
        let mut line = texts.t("lastObserved", &[("t", &ago(at))]);
        if a.stale { line = format!("{line} · {}", texts.t("lastReading", &[])); }
        state.push(line);
    }
    if let Some(code) = a.error.as_deref().or(a.monitor_error.as_deref()) { state.push(error_text(texts, provider, code)); }
    state.push(texts.t(if a.monitor_connected { "tipMonitorOn" } else { "tipMonitorOff" }, &[]));

    let mut held = vec![];
    if let Some(c) = &a.credits {
        let v = if c == "unlimited" { texts.t("unlimited", &[]) } else { c.clone() };
        held.push(texts.t("credits", &[("v", &v)]));
    }
    held.extend(f::group_resets(&a.resets).iter().map(|r| f::reset_text(lang, r, now)));

    [who, usage, state, held].into_iter()
        .map(|g| g.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>())
        .filter(|g| !g.is_empty())
        .map(|g| g.join("\n"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// An error code in words (`errorText` in i18n.js), for notices such as a failed monitoring
/// sign-in.
pub fn error_text(texts: &dyn Texts, provider: &str, code: &str) -> String {
    let cli = match provider {
        "Claude" => "Claude Code",
        other => other,
    };
    let vars = [("provider", cli)];
    match code {
        c if c.starts_with("monitor_") => texts.t(c, &[]),
        "token_expired" | "token_invalid" | "bad_response" => texts.t(code, &vars),
        "idle" => texts.t("idle", &[]),
        "rate_limited" => texts.t("rateLimited", &[]),
        "active" => texts.t("activeCantRemove", &[]),
        "CLI not found" => texts.t("cliMissing", &[]),
        c => match c.strip_prefix("http_").filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            Some(n) => texts.t("http", &[("code", n)]),
            None => c.to_string(),
        },
    }
}

/// The health dot's tooltip.
pub fn health_tip(texts: &dyn Texts, problems: usize) -> String {
    texts.t("healthIssues", &[("n", &problems.to_string())])
}

/// What the widget shows for a snapshot (`renderCompact` in main.js): one ring per provider (the
/// account in use, else its first one), or a line of text while loading, without accounts, or
/// while a notice (`toast`) is up.
pub fn view(snap: &Snapshot, toast: Option<&str>, health_problems: usize, texts: &dyn Texts) -> View {
    let mut details = vec![];
    let mut overlap = vec![];
    let items = if snap.updated_at.is_none() {
        vec![Item::Text { text: texts.t("loading", &[]), dim: true }]
    } else if snap.accounts.is_empty() {
        vec![Item::Text { text: texts.t("noAccounts", &[]), dim: true }]
    } else if let Some(toast) = toast {
        vec![Item::Text { text: toast.to_string(), dim: false }]
    } else {
        Provider::ALL
            .iter()
            .filter_map(|&p| {
                snap.accounts
                    .iter()
                    .find(|a| a.provider == p && a.active)
                    .or_else(|| snap.accounts.iter().find(|a| a.provider == p))
            })
            .map(|a| {
                let percent = if a.stale || a.error.is_some() { None } else { peak(a) };
                details.push(account_detail(texts, a, chrono::Utc::now()));
                overlap.push(a.overlap.len() > 1);
                Item::Chip { provider: a.provider, percent, tip: chip_tip(texts, a.provider, percent) }
            })
            .collect()
    };
    details.resize(items.len(), String::new());
    overlap.resize(items.len(), false);
    View { items, health_problems, label: texts.t("title", &[]), details, overlap }
}

/// A box in DIPs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Box2 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Box2 {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The whole widget's width (what dock.rs docks), in DIPs.
    pub width: f32,
    /// The hover/pressed box: the window's height less the inset above and below.
    pub frame: Box2,
    /// Per item, where it is drawn: a ring's 24×24 box, or a line of text's box.
    pub items: Vec<Box2>,
    pub dot: Option<Box2>,
    /// Per item, whether its text was cut short (with an ellipsis) to fit the room on the
    /// taskbar; its tooltip then says it in full.
    pub cut: Vec<bool>,
}

/// Lays the view out in a window `height` DIPs tall. `measure` gives a text's width at the
/// widget's font (DirectWrite on Windows).
#[cfg(test)]
pub fn layout(view: &View, height: f32, measure: &dyn Fn(&str) -> f32) -> Layout {
    layout_within(view, height, f32::INFINITY, measure)
}

/// As `layout`, at most `room` DIPs wide: text (a long notice) is cut short to fit, so the
/// widget never reaches over the taskbar's other buttons. Rings are never cut.
pub fn layout_within(view: &View, height: f32, room: f32, measure: &dyn Fn(&str) -> f32) -> Layout {
    let frame_h = (height - 2.0 * INSET_Y).max(RING_BOX);
    let mid = INSET_Y + frame_h / 2.0;
    let text_only = view.items.iter().all(|i| matches!(i, Item::Text { .. }));
    let pad = if text_only && !view.items.is_empty() { PAD_TEXT } else { PAD_CHIPS };
    let mut sizes: Vec<(f32, f32)> = view
        .items
        .iter()
        .map(|item| match item {
            Item::Chip { .. } => (RING_BOX, RING_BOX),
            Item::Text { text, .. } => (measure(text).ceil(), (FONT_SIZE * 1.35).round()),
        })
        .collect();
    let gaps = GAP * view.items.len().saturating_sub(1) as f32;
    let natural = 2.0 * (BORDER + pad) + gaps + sizes.iter().map(|s| s.0).sum::<f32>();
    let mut excess = (natural - room).max(0.0);
    let mut cut = vec![false; sizes.len()];
    // The last text gives way first (there is only ever one today).
    for (i, item) in view.items.iter().enumerate().rev() {
        if excess <= 0.0 {
            break;
        }
        if matches!(item, Item::Text { .. }) {
            let take = excess.min(sizes[i].0).ceil();
            sizes[i].0 -= take;
            excess -= take;
            cut[i] = true;
        }
    }
    let mut x = BORDER + pad;
    let mut boxes = Vec::with_capacity(view.items.len());
    for (i, (w, h)) in sizes.into_iter().enumerate() {
        if i > 0 {
            x += GAP;
        }
        boxes.push(Box2 { x, y: mid - h / 2.0, w, h });
        x += w;
    }
    let width = x + pad + BORDER;
    let frame = Box2 { x: 0.0, y: INSET_Y, w: width, h: frame_h };
    let dot = (view.health_problems > 0).then(|| Box2 {
        x: width - BORDER - 2.0 - DOT_SIZE,
        y: INSET_Y + BORDER + 2.0,
        w: DOT_SIZE,
        h: DOT_SIZE,
    });
    Layout { width, frame, items: boxes, dot, cut }
}

/// The amber badge on a ring whose account is in use on several machines at once: a dot at the
/// ring box's lower right, clear of the health dot (the widget's upper right). Center and radius.
pub fn overlap_badge(ring: &Box2) -> ((f32, f32), f32) {
    let r = BADGE_SIZE / 2.0;
    ((ring.x + ring.w - r, ring.y + ring.h - r), r)
}

/// DIPs to physical pixels for a monitor at `dpi` (96 = 100%). Window sizes round up so nothing
/// is clipped.
pub fn scale(dpi: u32) -> f32 {
    dpi.max(96) as f32 / 96.0
}

pub fn physical(dips: f32, dpi: u32) -> i32 {
    (dips * scale(dpi) - 0.001).ceil() as i32
}

/// A ring's whole tooltip: its headline ("Claude 42%"), then the account's details.
fn full_tip(tip: &str, detail: Option<&String>) -> String {
    match detail.filter(|d| !d.is_empty()) {
        Some(d) => format!("{tip}\n\n{d}"),
        None => tip.to_string(),
    }
}

/// Tooltip regions: each ring with its text, then the health dot.
pub fn tips(view: &View, layout: &Layout, texts: &dyn Texts) -> Vec<(Box2, String)> {
    let mut out: Vec<_> = view
        .items
        .iter()
        .zip(&layout.items)
        .enumerate()
        .filter_map(|(i, (item, b))| match item {
            Item::Chip { tip, .. } => Some((*b, full_tip(tip, view.details.get(i)))),
            // Cut short to fit: the tooltip says it all.
            Item::Text { text, .. } => layout.cut.get(i).copied().unwrap_or(false).then(|| (*b, text.clone())),
        })
        .collect();
    if let Some(dot) = layout.dot {
        // A bigger target than the 6 px dot itself.
        let hit = Box2 { x: dot.x - 4.0, y: dot.y - 4.0, w: dot.w + 8.0, h: dot.h + 8.0 };
        out.push((hit, health_tip(texts, view.health_problems)));
    }
    out
}

/// Everything the widget says, one line each: its tooltips, or its text. MCP's `inspect_window`
/// returns this as the window's text.
pub fn text(view: &View, texts: &dyn Texts) -> String {
    let mut lines: Vec<String> = view
        .items
        .iter()
        .enumerate()
        .map(|(n, i)| match i {
            Item::Chip { tip, .. } => full_tip(tip, view.details.get(n)),
            Item::Text { text, .. } => text.clone(),
        })
        .collect();
    if view.health_problems > 0 {
        lines.push(health_tip(texts, view.health_problems));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------------------------
// Provider marks: the SVG paths of src/marks.js, read at first use and turned into absolute
// segments the window builds Direct2D geometry from.

const MARKS_JS: &str = include_str!("../../src/marks.js");

/// The mark's drawn size in DIPs (styles.css `.chip .mark.claude` / `.codex`): the spark is all
/// thin rays and the Codex mark a solid shape, so they differ to balance their weight.
pub fn mark_size(p: Provider) -> f32 {
    match p {
        Provider::Claude => 14.0,
        Provider::Codex => 12.0,
    }
}

/// The marks' view box is 24 units square.
pub const MARK_VIEW: f32 = 24.0;

pub fn mark_path(p: Provider) -> &'static [Seg] {
    use std::sync::OnceLock;
    static MARKS: OnceLock<[Vec<Seg>; 2]> = OnceLock::new();
    let marks = MARKS.get_or_init(|| {
        let read = |key: &str| {
            let at = MARKS_JS.find(&format!("{key}: `")).unwrap_or(0);
            let rest = &MARKS_JS[at..];
            let d = rest.find(" d=\"").map(|i| &rest[i + 4..]).unwrap_or("");
            parse_path(&d[..d.find('"').unwrap_or(0)])
        };
        [read("claude"), read("codex")]
    });
    match p {
        Provider::Claude => &marks[0],
        Provider::Codex => &marks[1],
    }
}

/// One absolute path segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Quad(f32, f32, f32, f32),
    /// rx, ry, x-axis rotation (degrees), large arc, sweep (clockwise), end point.
    Arc { rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, x: f32, y: f32 },
    Close,
}

/// Parses SVG path data (all commands, relative and absolute, implicit repeats, compact numbers
/// such as "0 01-.104-.729") into absolute segments. Malformed data ends the path where it broke.
pub fn parse_path(d: &str) -> Vec<Seg> {
    let mut lx = Lexer { s: d.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // The last control point, for S/T reflections.
    let mut last_cubic: Option<(f32, f32)> = None;
    let mut last_quad: Option<(f32, f32)> = None;
    let mut cmd = b'M';
    loop {
        lx.skip();
        let Some(&c) = lx.s.get(lx.i) else { break };
        if c.is_ascii_alphabetic() {
            cmd = c;
            lx.i += 1;
        } else if cmd == b'Z' || cmd == b'z' {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let mut cubic = None;
        let mut quad = None;
        match cmd.to_ascii_uppercase() {
            b'Z' => {
                out.push(Seg::Close);
                (cx, cy) = (sx, sy);
            }
            b'M' => {
                let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                (cx, cy) = (ox + x, oy + y);
                (sx, sy) = (cx, cy);
                out.push(Seg::Move(cx, cy));
                // Further pairs are lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                (cx, cy) = (ox + x, oy + y);
                out.push(Seg::Line(cx, cy));
            }
            b'H' => {
                let Some(x) = lx.num() else { break };
                cx = ox + x;
                out.push(Seg::Line(cx, cy));
            }
            b'V' => {
                let Some(y) = lx.num() else { break };
                cy = oy + y;
                out.push(Seg::Line(cx, cy));
            }
            b'C' | b'S' => {
                let (x1, y1) = if cmd.to_ascii_uppercase() == b'C' {
                    let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                    (ox + x, oy + y)
                } else {
                    last_cubic.map_or((cx, cy), |(px, py)| (2.0 * cx - px, 2.0 * cy - py))
                };
                let (Some(x2), Some(y2), Some(x), Some(y)) = (lx.num(), lx.num(), lx.num(), lx.num()) else { break };
                let (x2, y2) = (ox + x2, oy + y2);
                (cx, cy) = (ox + x, oy + y);
                out.push(Seg::Cubic(x1, y1, x2, y2, cx, cy));
                cubic = Some((x2, y2));
            }
            b'Q' | b'T' => {
                let (x1, y1) = if cmd.to_ascii_uppercase() == b'Q' {
                    let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                    (ox + x, oy + y)
                } else {
                    last_quad.map_or((cx, cy), |(px, py)| (2.0 * cx - px, 2.0 * cy - py))
                };
                let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                (cx, cy) = (ox + x, oy + y);
                out.push(Seg::Quad(x1, y1, cx, cy));
                quad = Some((x1, y1));
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rot)) = (lx.num(), lx.num(), lx.num()) else { break };
                let (Some(large), Some(sweep)) = (lx.flag(), lx.flag()) else { break };
                let (Some(x), Some(y)) = (lx.num(), lx.num()) else { break };
                (cx, cy) = (ox + x, oy + y);
                out.push(Seg::Arc { rx: rx.abs(), ry: ry.abs(), rot, large, sweep, x: cx, y: cy });
            }
            _ => break,
        }
        last_cubic = cubic;
        last_quad = quad;
    }
    out
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
            self.i += 1;
        }
    }
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
    fn num(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let at = |i: usize| self.s.get(i).copied();
        let mut i = self.i;
        if matches!(at(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let mut dot = false;
        let mut digits = false;
        while let Some(c) = at(i) {
            if c.is_ascii_digit() {
                digits = true;
            } else if c == b'.' && !dot {
                dot = true;
            } else {
                break;
            }
            i += 1;
        }
        if digits && matches!(at(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(at(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if at(j).is_some_and(|c| c.is_ascii_digit()) {
                while at(j).is_some_and(|c| c.is_ascii_digit()) {
                    j += 1;
                }
                i = j;
            }
        }
        if !digits {
            return None;
        }
        self.i = i;
        std::str::from_utf8(&self.s[start..i]).ok()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageWindow;

    struct En;
    impl Texts for En {
        fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
            crate::i18n::t("en", key, vars)
        }
    }

    fn account(provider: Provider, id: &str, active: bool, used: &[f64]) -> AccountUsage {
        let mut a: AccountUsage = serde_json::from_value(serde_json::json!({
            "id": id, "provider": provider, "active": active, "windows": [], "email": null, "alias": null,
            "plan": null, "org": null, "sources": [], "credits": null, "resets": [], "error": null, "stale": false,
            "fetched_at": null, "monitor_connected": true,
        }))
        .unwrap();
        a.windows = used
            .iter()
            .map(|&p| UsageWindow { kind: "weekly".into(), window_seconds: None, used_percent: p, resets_at: None })
            .collect();
        a
    }

    fn snap(accounts: Vec<AccountUsage>) -> Snapshot {
        Snapshot { accounts, updated_at: Some(chrono::Utc::now()), ..Default::default() }
    }

    /// Text is 7 DIPs a character, so widths are easy to check.
    fn measure(s: &str) -> f32 {
        s.chars().count() as f32 * 7.0
    }

    #[test]
    fn the_arc_starts_at_twelve_and_runs_clockwise() {
        assert_eq!(arc_degrees(0.0), (-90.0, 0.0));
        assert_eq!(arc_degrees(25.0), (-90.0, 90.0));
        assert_eq!(arc_degrees(50.0), (-90.0, 180.0));
        assert_eq!(arc_degrees(100.0), (-90.0, 360.0));
        // Over the limit stays a full ring; nonsense draws nothing.
        assert_eq!(arc_degrees(140.0), (-90.0, 360.0));
        assert_eq!(arc_degrees(-5.0), (-90.0, 0.0));
        assert_eq!(arc_degrees(f64::NAN), (-90.0, 0.0));
        let (x, y) = point_on_ring((12.0, 12.0), RING_RADIUS, -90.0);
        assert!((x - 12.0).abs() < 1e-4 && (y - (12.0 - RING_RADIUS)).abs() < 1e-4);
        let (x, y) = point_on_ring((12.0, 12.0), RING_RADIUS, 0.0);
        assert!((x - (12.0 + RING_RADIUS)).abs() < 1e-4 && (y - 12.0).abs() < 1e-4);
    }

    #[test]
    fn the_ring_matches_the_web_widget() {
        assert!((RING_RADIUS - 9.6).abs() < 1e-5);
        assert!((RING_STROKE - 2.56).abs() < 1e-5);
    }

    #[test]
    fn ring_colors_follow_the_thresholds_and_theme() {
        for theme in [Theme::Light, Theme::Dark] {
            let p = palette(theme);
            assert_eq!(usage_color(&p, 10.0), p.ok);
            assert_eq!(usage_color(&p, 69.9), p.ok);
            assert_eq!(usage_color(&p, 70.0), p.warn);
            assert_eq!(usage_color(&p, 90.0), p.bad);
            assert_eq!(usage_color(&p, 120.0), p.bad);
        }
        assert_eq!(Theme::of_taskbar(true), Theme::Light);
        assert_eq!(palette(Theme::Dark).ok, rgb(0x6ccb5f));
        assert_eq!(palette(Theme::Light).ok, rgb(0x0f7b0f));
        assert_eq!(palette(Theme::Light).bad, rgb(0xc42b1c));
        assert_eq!(palette(Theme::Dark).text, rgb(0xffffff));
        assert!((palette(Theme::Light).text.a - 0.896).abs() < 1e-6);
    }

    #[test]
    fn an_account_on_several_machines_gets_a_badge_and_says_why() {
        let mut codex = account(Provider::Codex, "c1", true, &[10.0]);
        codex.overlap = vec!["DESK-1".into(), "LAPTOP-2".into()];
        let v = view(&snap(vec![account(Provider::Claude, "a1", true, &[5.0]), codex]), None, 0, &En);
        assert_eq!(v.overlap, [false, true]);
        assert!(!v.details[0].contains("machines"));
        assert!(v.details[1].contains("In use on 2 machines at once: DESK-1, LAPTOP-2"), "{}", v.details[1]);
        assert!(v.details[1].contains("Limits drain about 2× as fast"), "{}", v.details[1]);
        // Lower right of the ring, clear of the health dot at the widget's upper right.
        let l = layout(&v, 48.0, &measure);
        let ((x, y), r) = overlap_badge(&l.items[1]);
        assert!(x > l.items[1].x + l.items[1].w / 2.0 && y > l.items[1].y + l.items[1].h / 2.0 && r > 0.0);
    }

    #[test]
    fn shows_the_account_in_use_for_each_provider() {
        let s = snap(vec![
            account(Provider::Codex, "c1", false, &[10.0]),
            account(Provider::Claude, "a1", false, &[5.0]),
            account(Provider::Claude, "a2", true, &[30.0, 72.4]),
        ]);
        let v = view(&s, None, 0, &En);
        assert_eq!(v.items, vec![
            Item::Chip { provider: Provider::Claude, percent: Some(72.4), tip: "Claude 72%".into() },
            Item::Chip { provider: Provider::Codex, percent: Some(10.0), tip: "Codex 10%".into() },
        ]);
        assert_eq!(v.label, crate::i18n::t("en", "title", &[]));
    }

    #[test]
    fn stale_or_failing_usage_is_unknown() {
        let mut a = account(Provider::Claude, "a", true, &[40.0]);
        a.stale = true;
        let mut b = account(Provider::Codex, "b", true, &[40.0]);
        b.error = Some("http_500".into());
        let v = view(&snap(vec![a, b]), None, 0, &En);
        let unknown = crate::i18n::t("en", "usageUnknown", &[]);
        assert_eq!(v.items[0], Item::Chip { provider: Provider::Claude, percent: None, tip: format!("Claude — {unknown}") });
        assert!(matches!(v.items[1], Item::Chip { percent: None, .. }));
    }

    #[test]
    fn loading_empty_and_notices_are_text() {
        let loading = Snapshot::default();
        assert_eq!(view(&loading, None, 0, &En).items, vec![Item::Text { text: crate::i18n::t("en", "loading", &[]), dim: true }]);
        assert_eq!(view(&snap(vec![]), None, 0, &En).items, vec![Item::Text { text: crate::i18n::t("en", "noAccounts", &[]), dim: true }]);
        let s = snap(vec![account(Provider::Claude, "a", true, &[1.0])]);
        assert_eq!(view(&s, Some("Updated to 1.2"), 0, &En).items, vec![Item::Text { text: "Updated to 1.2".into(), dim: false }]);
    }

    #[test]
    fn layout_widths_match_the_web_widget() {
        let s = snap(vec![account(Provider::Claude, "a", true, &[1.0]), account(Provider::Codex, "b", true, &[1.0])]);
        // Two rings: border, padding 6, 24, gap 4, 24, padding 6, border.
        let l = layout(&view(&s, None, 0, &En), 48.0, &measure);
        assert_eq!(l.width, 1.0 + 6.0 + 24.0 + 4.0 + 24.0 + 6.0 + 1.0);
        assert_eq!(l.items[0], Box2 { x: 7.0, y: 12.0, w: 24.0, h: 24.0 });
        assert_eq!(l.items[1].x, 35.0);
        assert_eq!(l.frame, Box2 { x: 0.0, y: 4.0, w: 66.0, h: 40.0 });
        assert_eq!(l.dot, None);
        // One ring.
        let one = snap(vec![account(Provider::Codex, "b", true, &[1.0])]);
        assert_eq!(layout(&view(&one, None, 0, &En), 48.0, &measure).width, 38.0);
        // Text: padding 8 each side.
        let t = layout(&view(&snap(vec![]), None, 0, &En), 48.0, &measure);
        let text = crate::i18n::t("en", "noAccounts", &[]);
        assert_eq!(t.width, 1.0 + 8.0 + measure(&text) + 8.0 + 1.0);
        assert_eq!(t.cut, vec![false]);
    }

    #[test]
    fn a_long_notice_is_cut_to_the_room_and_tipped_in_full() {
        let s = snap(vec![account(Provider::Claude, "a", true, &[1.0])]);
        let notice = "Monitoring sign-in did not complete. Try connecting again.";
        let v = view(&s, Some(notice), 0, &En);
        let full = layout(&v, 48.0, &measure);
        assert!(full.width > 120.0);
        assert!(tips(&v, &full, &En).is_empty());
        let l = layout_within(&v, 48.0, 120.0, &measure);
        assert_eq!(l.width, 120.0);
        assert_eq!(l.cut, vec![true]);
        assert_eq!(l.items[0].w, 120.0 - 2.0 * (1.0 + 8.0));
        assert_eq!(tips(&v, &l, &En), vec![(l.items[0], notice.to_string())]);
        // Room to spare cuts nothing.
        assert_eq!(layout_within(&v, 48.0, 10_000.0, &measure), full);
    }

    #[test]
    fn rings_are_never_cut() {
        let s = snap(vec![account(Provider::Claude, "a", true, &[1.0]), account(Provider::Codex, "b", true, &[1.0])]);
        let l = layout_within(&view(&s, None, 0, &En), 48.0, 30.0, &measure);
        assert_eq!(l.width, 66.0);
        assert_eq!(l.cut, vec![false, false]);
    }

    #[test]
    fn health_problems_add_a_dot_and_a_tip() {
        let s = snap(vec![account(Provider::Claude, "a", true, &[50.0])]);
        let v = view(&s, None, 2, &En);
        let l = layout(&v, 48.0, &measure);
        let dot = l.dot.unwrap();
        assert_eq!((dot.w, dot.h), (DOT_SIZE, DOT_SIZE));
        assert!(dot.x + dot.w <= l.width - BORDER && dot.y >= INSET_Y);
        let tips = tips(&v, &l, &En);
        assert_eq!(tips.len(), 2);
        // The ring's tooltip: its headline, then the account's details.
        assert_eq!(tips[0].0, l.items[0]);
        assert_eq!(tips[0].1, format!("Claude 50%\n\n{}", v.details[0]));
        assert_eq!(tips[1].1, crate::i18n::t("en", "healthIssues", &[("n", "2")]));
        assert!(tips[1].0.contains(dot.x + 3.0, dot.y + 3.0));
        assert_eq!(text(&v, &En), format!("{}\n{}", tips[0].1, tips[1].1));
    }

    #[test]
    fn the_ring_tooltip_says_what_the_full_card_says() {
        use chrono::TimeZone;
        let en = |k: &str, v: &[(&str, &str)]| crate::i18n::t("en", k, v);
        let now = chrono::Utc.with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap();
        let mut a: AccountUsage = serde_json::from_value(serde_json::json!({
            "id": "a1", "provider": "claude", "active": true, "email": "max@example.invalid", "alias": "Work",
            "plan": "Max 20x", "org": null, "sources": ["Windows"], "credits": null, "error": null, "stale": false,
            "fetched_at": now - chrono::Duration::minutes(3), "monitor_connected": true, "leased": "nightly build",
            "remote": [{ "machine": "LAPTOP-2", "where": ["WSL: Ubuntu"], "seen_at": now, "live": true }],
            "resets": [{ "kind": "weekly", "count": 1, "usable": true, "title": null, "expires_at": null, "next_available_at": null }],
            "windows": [
                { "kind": "weekly", "used_percent": 77.0, "window_seconds": 604800, "resets_at": now + chrono::Duration::hours(52) },
                { "kind": "weekly_opus", "used_percent": 40.0, "window_seconds": 604800, "resets_at": null },
            ],
        })).unwrap();
        a.runs_out_at = Some(now + chrono::Duration::hours(30));
        let d = account_detail(&En, &a, now);
        let groups: Vec<&str> = d.split("\n\n").collect();
        assert_eq!(groups.len(), 4, "{d}");
        assert_eq!(groups[0].lines().collect::<Vec<_>>(), [
            "Work (max@example.invalid)".to_string(),
            "Claude · Max 20x".into(),
            format!("{}: Windows, LAPTOP-2 (WSL: Ubuntu)", en("inUse", &[])),
            en("leased", &[("purpose", "nightly build")]),
        ]);
        let reset = crate::i18n_format::window_reset_text("en", &a.windows[0], false, a.fetched_at, now);
        assert!(reset.starts_with(&en("resetsIn", &[("t", "2d 4h")])) && reset.contains(" · "), "relative and absolute: {reset}");
        assert!(groups[1].starts_with(&format!("{}: 77% {} · {reset}", en("weekly", &[]), en("used", &[]))), "{}", groups[1]);
        assert!(groups[1].contains(&format!("{}: 40%", en("weekly_opus", &[]))), "model windows too: {}", groups[1]);
        assert!(groups[1].contains(&en("runsOut", &[("at", &crate::i18n_format::day_time("en", now + chrono::Duration::hours(30)))])));
        assert_eq!(groups[2], format!("{}\n{}", en("lastObserved", &[("t", "3m")]), en("tipMonitorOn", &[])));
        assert!(groups[3].contains(&en("resetUsableNow", &[])));
        // A failing, unmonitored account says why, and leaves out what it doesn't have.
        let mut b = a.clone();
        b.alias = None;
        b.leased = None;
        b.remote.clear();
        b.resets.clear();
        b.active = false;
        b.monitor_connected = false;
        b.error = Some("token_expired".into());
        let d = account_detail(&En, &b, now);
        assert!(d.starts_with("max@example.invalid\nClaude · Max 20x\n\n"), "{d}");
        assert!(d.contains(&en("token_expired", &[("provider", "Claude Code")])) && d.contains(&en("tipMonitorOff", &[])));
        assert!(!d.ends_with('\n') && !d.contains("\n\n\n"));
    }

    #[test]
    fn error_codes_read_as_in_the_flyout() {
        let en = |k: &str, v: &[(&str, &str)]| crate::i18n::t("en", k, v);
        assert_eq!(error_text(&En, "", "http_503"), en("http", &[("code", "503")]));
        assert_eq!(error_text(&En, "Claude", "token_expired"), en("token_expired", &[("provider", "Claude Code")]));
        assert_eq!(error_text(&En, "", "monitor_network"), en("monitor_network", &[]));
        assert_eq!(error_text(&En, "", "rate_limited"), en("rateLimited", &[]));
        assert_eq!(error_text(&En, "", "http_"), "http_");
        assert_eq!(error_text(&En, "", "connection reset"), "connection reset");
    }

    #[test]
    fn dpi_scaling_rounds_up_to_whole_pixels() {
        assert_eq!(scale(96), 1.0);
        assert_eq!(scale(144), 1.5);
        assert_eq!(scale(240), 2.5);
        assert_eq!(scale(0), 1.0);
        assert_eq!(physical(66.0, 96), 66);
        assert_eq!(physical(66.0, 120), 83); // 82.5
        assert_eq!(physical(66.0, 144), 99);
        assert_eq!(physical(66.0, 192), 132);
        assert_eq!(physical(66.0, 240), 165);
        assert_eq!(physical(48.0, 168), 84);
    }

    #[test]
    fn a_taller_taskbar_centers_the_rings() {
        let s = snap(vec![account(Provider::Claude, "a", true, &[1.0])]);
        let l = layout(&view(&s, None, 0, &En), 72.0, &measure);
        assert_eq!(l.items[0].y, 24.0);
        assert_eq!(l.frame.h, 64.0);
    }

    #[test]
    fn parses_svg_paths() {
        assert_eq!(parse_path("M1 2L3 4H5V6Z"), vec![Seg::Move(1.0, 2.0), Seg::Line(3.0, 4.0), Seg::Line(5.0, 4.0), Seg::Line(5.0, 6.0), Seg::Close]);
        // Relative, implicit repeats after a move, compact numbers.
        assert_eq!(parse_path("m1 1 2 0-.5.5z"), vec![Seg::Move(1.0, 1.0), Seg::Line(3.0, 1.0), Seg::Line(2.5, 1.5), Seg::Close]);
        // Arc flags packed against the numbers: "a2.97 2.97 0 01-.104-.729".
        assert_eq!(parse_path("M10 10a2.97 2.97 0 01-.104-.729"), vec![
            Seg::Move(10.0, 10.0),
            Seg::Arc { rx: 2.97, ry: 2.97, rot: 0.0, large: false, sweep: true, x: 9.896, y: 9.271 },
        ]);
        // Smooth curves reflect the previous control point.
        let s = parse_path("M0 0C1 1 2 1 3 0S5 -1 6 0");
        assert_eq!(s[2], Seg::Cubic(4.0, -1.0, 5.0, -1.0, 6.0, 0.0));
        assert_eq!(parse_path("M0 0q1 1 2 0t2 0")[2], Seg::Quad(3.0, -1.0, 4.0, 0.0));
        assert_eq!(parse_path("M1e1 2E-1"), vec![Seg::Move(10.0, 0.2)]);
    }

    #[test]
    fn reads_both_marks_from_the_frontend() {
        for p in Provider::ALL {
            let path = mark_path(p);
            assert!(path.len() > 20, "{p:?}: {}", path.len());
            assert!(matches!(path[0], Seg::Move(..)));
            // Every point stays inside the 24-unit view box.
            for seg in path {
                let pts: Vec<(f32, f32)> = match *seg {
                    Seg::Move(x, y) | Seg::Line(x, y) => vec![(x, y)],
                    Seg::Cubic(a, b, c, d, e, f) => vec![(a, b), (c, d), (e, f)],
                    Seg::Quad(a, b, c, d) => vec![(a, b), (c, d)],
                    Seg::Arc { x, y, .. } => vec![(x, y)],
                    Seg::Close => vec![],
                };
                for (x, y) in pts {
                    assert!((-0.5..=24.5).contains(&x) && (-0.5..=24.5).contains(&y), "{p:?} {seg:?}");
                }
            }
        }
        assert!(mark_path(Provider::Codex).iter().filter(|s| matches!(s, Seg::Close)).count() >= 3);
    }
}
