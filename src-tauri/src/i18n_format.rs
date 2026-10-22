//! The locale formatting src/i18n.js does with `Intl` (durations, weekdays, dates, money), for text
//! the app words itself without a webview: notifications (see `alerts`) and the native widget.
//! Translations come from `i18n`; this only formats numbers and dates around them.
//!
//! English matches the webview's output exactly ("3d 4h", "Thu 3 PM", "Oct 22", "$1,234.5").
//! The other nine languages use small CLDR-like tables, close to but not always identical to
//! what `Intl` prints for every regional variant:
//! - the language decides the pattern, not the region (en-GB still gets "Oct 22", "3 PM");
//! - a time zone is named by its offset ("GMT+2", "UTC"), never an abbreviation like "PDT";
//! - `money` follows the app language, where the webview used the system's default locale.
//!
//! Times are shown in the local time zone, as the webview did; each formatter has an `_in`
//! variant taking any time zone, which the tests use.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike, Utc};

use crate::i18n::t;
use crate::model::ResetGrant;

/// The two-letter language of a code such as "de" or "pt-BR".
fn base(lang: &str) -> &str {
    lang.get(..2).unwrap_or("en")
}

/// `Math.round`: halves go up.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// A JavaScript number as `String(n)` writes it: no ".0" on whole numbers.
pub fn js_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{}", x as i64) } else { format!("{x}") }
}

/// Narrow day, hour and minute units (`Intl.NumberFormat` unit style "narrow").
fn units(lang: &str) -> [(&'static str, &'static str); 3] {
    match base(lang) {
        "de" => [("", " T"), ("", " Std."), ("", " Min.")],
        "fr" => [("", "j"), ("", "h"), ("", "min")],
        "es" | "pt" => [("", "d"), ("", "h"), ("", "min")],
        "it" => [("", "g"), ("", "h"), ("", "min")],
        "ja" => [("", "日"), ("", "時間"), ("", "分")],
        "zh" => [("", "天"), ("", "小时"), ("", "分钟")],
        "ko" => [("", "일"), ("", "시간"), ("", "분")],
        "ru" => [("", " дн."), ("", " ч"), ("", " мин")],
        _ => [("", "d"), ("", "h"), ("", "m")],
    }
}

/// Compact duration such as "3d 4h" or "4h 47m", formatted for the language (i18n.js `duration`).
pub fn duration(lang: &str, total_minutes: i64) -> String {
    let [day, hour, min] = units(lang);
    let unit = |(pre, post): (&str, &str), n: i64| format!("{pre}{n}{post}");
    let total = total_minutes.max(0);
    let (d, h, m) = (total / 1440, (total % 1440) / 60, total % 60);
    if d > 0 {
        return if h > 0 { format!("{} {}", unit(day, d), unit(hour, h)) } else { unit(day, d) };
    }
    if h > 0 {
        return if m > 0 { format!("{} {}", unit(hour, h), unit(min, m)) } else { unit(hour, h) };
    }
    unit(min, m.max(1))
}

/// The time until `at`, at least a minute (alerts.js `until`).
pub fn until(lang: &str, at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mins = js_round((at - now).num_milliseconds() as f64 / 60000.0).max(1.0);
    duration(lang, mins as i64)
}

fn weekdays(lang: &str) -> [&'static str; 7] {
    match base(lang) {
        "de" => ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."],
        "fr" => ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
        "es" => ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
        "it" => ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
        "pt" => ["seg.", "ter.", "qua.", "qui.", "sex.", "sáb.", "dom."],
        "ja" => ["月", "火", "水", "木", "金", "土", "日"],
        "zh" => ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
        "ko" => ["월", "화", "수", "목", "금", "토", "일"],
        "ru" => ["пн", "вт", "ср", "чт", "пт", "сб", "вс"],
        _ => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    }
}

fn months(lang: &str) -> [&'static str; 12] {
    match base(lang) {
        "de" => ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."],
        "fr" => ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
        "es" => ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"],
        "it" => ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"],
        "pt" => ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."],
        "ru" => ["янв.", "февр.", "мар.", "апр.", "мая", "июн.", "июл.", "авг.", "сент.", "окт.", "нояб.", "дек."],
        _ => ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    }
}

/// The short weekday of a moment in a time zone ("Thu").
fn weekday_in<Tz: TimeZone>(lang: &str, at: &DateTime<Tz>) -> &'static str {
    weekdays(lang)[at.weekday().num_days_from_monday() as usize]
}

/// The short weekday of a moment, locally, or in UTC for a calendar day (digest `renewDay`).
pub fn weekday(lang: &str, at: DateTime<Utc>, utc: bool) -> String {
    if utc { weekday_in(lang, &at).to_string() } else { weekday_in(lang, &at.with_timezone(&Local)).to_string() }
}

/// "Oct 22" for a calendar day (i18n.js `shortDate`).
pub fn short_date_ymd(lang: &str, day: NaiveDate) -> String {
    let (m, d) = (day.month() as usize, day.day());
    let name = months(lang)[m - 1];
    match base(lang) {
        "de" => format!("{d}. {name}"),
        "fr" | "es" | "it" | "ru" => format!("{d} {name}"),
        "pt" => format!("{d} de {name}"),
        "ja" | "zh" => format!("{m}月{d}日"),
        "ko" => format!("{m}월 {d}일"),
        _ => format!("{name} {d}"),
    }
}

/// "Oct 22", local (i18n.js `shortDate`).
pub fn short_date(lang: &str, at: DateTime<Utc>) -> String {
    short_date_in(lang, &at.with_timezone(&Local))
}

pub fn short_date_in<Tz: TimeZone>(lang: &str, at: &DateTime<Tz>) -> String {
    short_date_ymd(lang, at.date_naive())
}

/// 12-hour clock parts: (hour 1-12, is afternoon).
fn twelve(hour: u32) -> (u32, bool) {
    (if hour % 12 == 0 { 12 } else { hour % 12 }, hour >= 12)
}

/// A weekday and hour, "Thu 3 PM", local (i18n.js `dayTime`).
pub fn day_time(lang: &str, at: DateTime<Utc>) -> String {
    day_time_in(lang, &at.with_timezone(&Local))
}

pub fn day_time_in<Tz: TimeZone>(lang: &str, at: &DateTime<Tz>) -> String {
    let wd = weekday_in(lang, at);
    let hour = at.hour();
    let (h12, pm) = twelve(hour);
    match base(lang) {
        "de" => format!("{wd}, {hour:02} Uhr"),
        "fr" => format!("{wd} {hour:02} h"),
        "es" | "pt" | "ru" => format!("{wd}, {hour}"),
        "it" => format!("{wd} {hour:02}"),
        "ja" => format!("{hour}時 ({wd})"),
        "zh" => format!("{wd} {hour}时"),
        "ko" => format!("({wd}) {} {h12}시", if pm { "오후" } else { "오전" }),
        _ => format!("{wd} {h12} {}", if pm { "PM" } else { "AM" }),
    }
}

/// The time zone named by its offset, as `timeZoneName: "short"` does outside the US.
fn zone_name(offset_secs: i32) -> String {
    if offset_secs == 0 {
        return "UTC".into();
    }
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let (h, m) = (offset_secs.abs() / 3600, offset_secs.abs() % 3600 / 60);
    if m == 0 { format!("GMT{sign}{h}") } else { format!("GMT{sign}{h}:{m:02}") }
}

/// "Oct 22, 3:05 PM GMT+2", local (i18n.js `resetDate`).
pub fn reset_date(lang: &str, at: DateTime<Utc>) -> String {
    reset_date_in(lang, &at.with_timezone(&Local))
}

pub fn reset_date_in<Tz: TimeZone>(lang: &str, at: &DateTime<Tz>) -> String {
    use chrono::Offset;
    let zone = zone_name(at.offset().fix().local_minus_utc());
    let date = short_date_in(lang, at);
    let (hour, minute) = (at.hour(), at.minute());
    let (h12, pm) = twelve(hour);
    match base(lang) {
        "en" => format!("{date}, {h12}:{minute:02} {} {zone}", if pm { "PM" } else { "AM" }),
        "ko" => format!("{date} {} {h12}:{minute:02} {zone}", if pm { "오후" } else { "오전" }),
        "ja" | "zh" => format!("{date} {hour}:{minute:02} {zone}"),
        _ => format!("{date}, {hour:02}:{minute:02} {zone}"),
    }
}

/// "Resets in 3d 4h", or that the reset passed and a refresh is due (i18n.js `resetsIn`).
pub fn resets_in(lang: &str, at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else { return t(lang, "resetTimeUnavailable", &[]) };
    if at <= now {
        return t(lang, "resetAwaitingRefresh", &[("d", &reset_date(lang, at))]);
    }
    let ms = (at - now).num_milliseconds();
    let mins = ((ms + 59_999) / 60_000).max(1);
    t(lang, "resetsIn", &[("t", &duration(lang, mins))])
}

/// A usage window's name: "Weekly", or "Limit · 5h" for a provider's own window (i18n.js `windowLabel`).
pub fn window_label(lang: &str, kind: &str, window_seconds: Option<i64>) -> String {
    if kind != "window" {
        return t(lang, kind, &[]);
    }
    match window_seconds.filter(|s| *s != 0) {
        Some(s) => format!("{} · {}", t(lang, "limit", &[]), duration(lang, js_round(s as f64 / 60.0) as i64)),
        None => t(lang, "limit", &[]),
    }
}

/// Identical resets merged (i18n.js `groupResets`): a total count, the soonest expiry, and how
/// many entries were merged.
#[derive(Debug, Clone, PartialEq)]
pub struct ResetGroup {
    pub kind: String,
    pub title: Option<String>,
    pub count: u32,
    pub usable: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub next_available_at: Option<DateTime<Utc>>,
    pub entries: u32,
}

pub fn group_resets(resets: &[ResetGrant]) -> Vec<ResetGroup> {
    let mut groups: Vec<ResetGroup> = Vec::new();
    for r in resets {
        let same = |g: &&mut ResetGroup| g.kind == r.kind && g.title.as_deref().unwrap_or("") == r.title.as_deref().unwrap_or("") && g.usable == r.usable;
        let Some(g) = groups.iter_mut().find(|g| same(g)) else {
            groups.push(ResetGroup {
                kind: r.kind.clone(), title: r.title.clone(), count: r.count, usable: r.usable,
                expires_at: r.expires_at, next_available_at: r.next_available_at, entries: 1,
            });
            continue;
        };
        g.count += r.count;
        g.entries += 1;
        if r.expires_at.is_some() && g.expires_at.is_none_or(|e| r.expires_at.unwrap() < e) { g.expires_at = r.expires_at; }
        if r.next_available_at.is_some() && g.next_available_at.is_none_or(|e| r.next_available_at.unwrap() < e) { g.next_available_at = r.next_available_at; }
    }
    // Usable first, then the ones expiring soonest (none last).
    groups.sort_by(|a, b| b.usable.cmp(&a.usable).then_with(|| match (a.expires_at, b.expires_at) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }));
    groups
}

/// One line describing a usage-limit reset or a group of identical ones (i18n.js `resetText`).
pub fn reset_text(lang: &str, r: &ResetGroup, now: DateTime<Utc>) -> String {
    let scope = match r.kind.as_str() { "session" => Some("resetSession"), "weekly" => Some("resetWeekly"), "full" => Some("resetFull"), _ => None };
    let name = match scope {
        Some(scope) => r.title.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| t(lang, scope, &[])),
        None => t(lang, "resetUnknown", &[]),
    };
    let mut parts = vec![if r.count > 1 { format!("{name} ×{}", r.count) } else { name }];
    if r.usable {
        parts.push(t(lang, "resetUsableNow", &[]));
    } else if r.count > 0 {
        parts.push(t(lang, "resetNotUsableNow", &[]));
    }
    if let Some(e) = r.expires_at {
        parts.push(t(lang, if r.entries > 1 { "resetNextExpires" } else { "resetExpires" }, &[("d", &short_date(lang, e))]));
    }
    if let (Some(next), 0) = (r.next_available_at, r.count) {
        let mins = js_round((next - now).num_milliseconds() as f64 / 60000.0).max(1.0);
        parts.push(t(lang, "resetNext", &[("t", &duration(lang, mins as i64))]));
    }
    parts.join(" · ")
}

/// "$1,234.5": dollars with at most two decimals, grouped for the language (alerts.js `money`).
pub fn money(lang: &str, v: f64) -> String {
    let (group, decimal, min_grouping) = match base(lang) {
        "de" | "it" | "pt" => (".", ",", 4),
        "es" => (".", ",", 5),
        "fr" => ("\u{202F}", ",", 4),
        "ru" => ("\u{A0}", ",", 4),
        _ => (",", ".", 4),
    };
    let cents = (v.abs() * 100.0).round() as u64;
    let (whole, frac) = ((cents / 100).to_string(), cents % 100);
    let mut grouped = String::new();
    if whole.len() >= min_grouping {
        for (i, c) in whole.chars().enumerate() {
            if i > 0 && (whole.len() - i) % 3 == 0 { grouped.push_str(group); }
            grouped.push(c);
        }
    } else {
        grouped = whole;
    }
    let frac = match frac { 0 => String::new(), f if f % 10 == 0 => format!("{decimal}{}", f / 10), f => format!("{decimal}{f:02}") };
    format!("{}${grouped}{frac}", if v < 0.0 && cents > 0 { "-" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<Utc> { s.parse().unwrap() }

    #[test]
    fn durations_match_intl_narrow_english() {
        assert_eq!(duration("en", 3 * 1440 + 4 * 60 + 5), "3d 4h");
        assert_eq!(duration("en", 1440), "1d");
        assert_eq!(duration("en", 4 * 60 + 47), "4h 47m");
        assert_eq!(duration("en", 120), "2h");
        assert_eq!(duration("en", 0), "1m");
        assert_eq!(duration("de", 4 * 60 + 47), "4 Std. 47 Min.");
        assert_eq!(duration("fr", 61), "1h 1min");
    }

    #[test]
    fn until_rounds_to_the_minute_and_is_never_zero() {
        let now = at("2026-10-22T10:00:00Z");
        assert_eq!(until("en", at("2026-10-22T10:00:29Z"), now), "1m");
        assert_eq!(until("en", at("2026-10-22T12:30:30Z"), now), "2h 31m");
        assert_eq!(until("en", at("2026-10-21T10:00:00Z"), now), "1m");
    }

    #[test]
    fn day_time_and_short_date_in_english() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let thu = at("2026-10-22T13:00:00Z").with_timezone(&tz);
        assert_eq!(day_time_in("en", &thu), "Thu 3 PM");
        assert_eq!(day_time_in("en", &at("2026-10-22T00:30:00Z")), "Thu 12 AM");
        assert_eq!(day_time_in("en", &at("2026-10-22T12:00:00Z")), "Thu 12 PM");
        assert_eq!(short_date_in("en", &thu), "Oct 22");
        assert_eq!(short_date_ymd("en", NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()), "Jan 5");
        assert_eq!(short_date_in("de", &thu), "22. Okt.");
        assert_eq!(day_time_in("de", &thu), "Do., 15 Uhr");
        // Local time decides the day: 23:30 UTC is Friday at UTC+2.
        assert_eq!(short_date_in("en", &at("2026-10-22T23:30:00Z").with_timezone(&tz)), "Oct 23");
    }

    #[test]
    fn reset_date_names_the_offset() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(reset_date_in("en", &at("2026-10-22T13:05:00Z").with_timezone(&tz)), "Oct 22, 3:05 PM GMT+2");
        assert_eq!(reset_date_in("en", &at("2026-10-22T09:05:00Z")), "Oct 22, 9:05 AM UTC");
        let india = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        assert!(reset_date_in("en", &at("2026-10-22T09:05:00Z").with_timezone(&india)).ends_with("GMT+5:30"));
    }

    #[test]
    fn resets_in_rounds_up() {
        let now = at("2026-10-22T10:00:00Z");
        assert_eq!(resets_in("en", Some(at("2026-10-22T10:00:01Z")), now), t("en", "resetsIn", &[("t", "1m")]));
        assert_eq!(resets_in("en", Some(at("2026-10-22T12:00:30Z")), now), t("en", "resetsIn", &[("t", "2h 1m")]));
        assert_eq!(resets_in("en", None, now), "Reset time unavailable");
        assert!(resets_in("en", Some(at("2026-10-22T09:00:00Z")), now).starts_with("Reset time passed"));
    }

    #[test]
    fn window_labels() {
        assert_eq!(window_label("en", "weekly", None), "Weekly");
        assert_eq!(window_label("en", "window", Some(18000)), "Limit · 5h");
        assert_eq!(window_label("en", "window", None), "Limit");
    }

    #[test]
    fn money_like_to_locale_string() {
        assert_eq!(money("en", 1234.5), "$1,234.5");
        assert_eq!(money("en", 200.0), "$200");
        assert_eq!(money("en", 19.999), "$20");
        assert_eq!(money("en", 0.05), "$0.05");
        assert_eq!(money("en", 1_234_567.0), "$1,234,567");
        assert_eq!(money("de", 1234.5), "$1.234,5");
        assert_eq!(money("es", 1234.0), "$1234");
    }

    #[test]
    fn groups_resets_usable_first_then_soonest() {
        let g = |kind: &str, usable: bool, count: u32, exp: Option<&str>| ResetGrant {
            kind: kind.into(), title: None, count, usable, expires_at: exp.map(at), next_available_at: None,
        };
        let groups = group_resets(&[
            g("full", false, 1, Some("2026-11-02T00:00:00Z")),
            g("full", false, 2, Some("2026-10-30T00:00:00Z")),
            g("session", true, 1, None),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, "session");
        assert_eq!((groups[1].count, groups[1].entries), (3, 2));
        assert_eq!(groups[1].expires_at, Some(at("2026-10-30T00:00:00Z")));
        let now = at("2026-10-22T10:00:00Z");
        assert_eq!(reset_text("en", &groups[0], now), "Session reset · usable now");
        assert!(reset_text("en", &groups[1], now).starts_with("Full reset ×3 · not usable now · next expires "));
    }

    #[test]
    fn js_numbers() {
        assert_eq!(js_number(2.0), "2");
        assert_eq!(js_number(2.5), "2.5");
        assert_eq!(js_round(2.5), 3.0);
    }
}
