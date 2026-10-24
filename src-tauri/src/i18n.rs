//! The frontend's translations (src/i18n.js), read by the backend too, so text the app shows or
//! sends without a webview (the native taskbar widget, its menu, notifications) uses the same
//! strings in the same language. One source: the table is parsed out of the JavaScript file at
//! first use, so a translation added there is here too.

use std::collections::HashMap;
use std::sync::OnceLock;

const SOURCE: &str = include_str!("../../src/i18n.js");

type Table = HashMap<String, HashMap<String, String>>;

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(SOURCE).unwrap_or_default())
}

/// Every language the frontend has, by code ("en", "de", ...).
pub fn languages() -> Vec<&'static str> {
    let mut list: Vec<_> = table().keys().map(String::as_str).collect();
    list.sort();
    list
}

/// The language code for a settings value: "auto" (or anything unknown) follows the system's
/// display language, falling back to English, as `setLanguage` does in the frontend.
pub fn resolve(pref: &str) -> String {
    resolve_with(pref, sys_locale::get_locale().as_deref())
}

pub fn resolve_with(pref: &str, system: Option<&str>) -> String {
    let known = |code: &str| table().contains_key(code);
    if known(pref) {
        return pref.to_string();
    }
    let base = system.unwrap_or("en").to_lowercase();
    let base = base.split(['-', '_']).next().unwrap_or("en");
    if known(base) { base.to_string() } else { "en".into() }
}

/// `t(key, vars)` from i18n.js: the language's text, else English's, else the key itself, with
/// `{name}` placeholders filled from `vars` (missing ones become empty).
pub fn t(lang: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let msg = table()
        .get(lang)
        .and_then(|m| m.get(key))
        .or_else(|| table().get("en").and_then(|m| m.get(key)))
        .map(String::as_str)
        .unwrap_or(key);
    fill(msg, vars)
}

fn fill(msg: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if !after[..close].is_empty() && after[..close].chars().all(|c| c.is_alphanumeric() || c == '_') => {
                let name = &after[..close];
                out.push_str(vars.iter().find(|(k, _)| *k == name).map(|(_, v)| *v).unwrap_or(""));
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Reads `const MESSAGES = { en: { key: "text", ... }, ... };` out of the JavaScript source.
fn parse(src: &str) -> Option<Table> {
    let start = src.find("const MESSAGES = {")? + "const MESSAGES = ".len();
    let mut p = Parser { s: &src[start..], i: 0 };
    p.expect('{')?;
    let mut table = Table::new();
    loop {
        p.skip();
        if p.eat('}') {
            return Some(table);
        }
        let lang = p.key()?;
        p.skip();
        p.expect(':')?;
        p.skip();
        p.expect('{')?;
        let entries = table.entry(lang).or_default();
        loop {
            p.skip();
            if p.eat('}') {
                break;
            }
            let key = p.key()?;
            p.skip();
            p.expect(':')?;
            p.skip();
            let value = p.string()?;
            entries.insert(key, value);
            p.skip();
            p.eat(',');
        }
        p.skip();
        p.eat(',');
    }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: char) -> Option<()> {
        self.eat(c).then_some(())
    }
    /// Whitespace and comments.
    fn skip(&mut self) {
        loop {
            let rest = &self.s[self.i..];
            let trimmed = rest.trim_start();
            self.i += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                self.i += trimmed.find('\n').unwrap_or(trimmed.len());
            } else if trimmed.starts_with("/*") {
                self.i += trimmed.find("*/").map_or(trimmed.len(), |e| e + 2);
            } else {
                return;
            }
        }
    }
    fn key(&mut self) -> Option<String> {
        if matches!(self.peek(), Some('"' | '\'')) {
            return self.string();
        }
        let rest = &self.s[self.i..];
        let len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).unwrap_or(rest.len());
        (len > 0).then(|| {
            self.i += len;
            rest[..len].to_string()
        })
    }
    fn string(&mut self) -> Option<String> {
        let quote = self.peek().filter(|c| matches!(c, '"' | '\''))?;
        self.i += 1;
        let mut out = String::new();
        let mut chars = self.s[self.i..].char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                c if c == quote => {
                    self.i += at + 1;
                    return Some(out);
                }
                '\\' => {
                    let (_, e) = chars.next()?;
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String = (0..4).filter_map(|_| chars.next().map(|(_, h)| h)).collect();
                            out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_language_of_the_frontend() {
        let langs = languages();
        for code in ["en", "pt", "es", "fr", "de", "it", "ja", "zh", "ko", "ru"] {
            assert!(langs.contains(&code), "{code} missing from {langs:?}");
        }
        // The table is complete enough to be the real one, not a partial parse.
        assert!(table()["en"].len() > 400, "{}", table()["en"].len());
        assert!(table()["de"].len() > 100);
    }

    #[test]
    fn translates_and_fills_placeholders() {
        assert_eq!(t("en", "loading", &[]), table()["en"]["loading"]);
        assert_eq!(t("en", "healthOk", &[]), "OK");
        assert_eq!(t("en", "updatedTo", &[("v", "1.2.3")]), table()["en"]["updatedTo"].replace("{v}", "1.2.3"));
        assert!(t("de", "refresh", &[]) != t("en", "refresh", &[]));
    }

    #[test]
    fn falls_back_to_english_then_the_key() {
        assert_eq!(t("xx", "loading", &[]), t("en", "loading", &[]));
        assert_eq!(t("en", "no_such_key", &[]), "no_such_key");
    }

    #[test]
    fn missing_placeholders_become_empty_and_braces_survive() {
        assert_eq!(fill("{a} and {b}", &[("a", "x")]), "x and ");
        assert_eq!(fill("{ not a var }", &[]), "{ not a var }");
    }

    #[test]
    fn resolves_auto_from_the_system_language() {
        assert_eq!(resolve_with("auto", Some("de-DE")), "de");
        assert_eq!(resolve_with("auto", Some("sv-SE")), "en");
        assert_eq!(resolve_with("ja", Some("de-DE")), "ja");
        assert_eq!(resolve_with("auto", None), "en");
    }

    #[test]
    fn parses_escapes_and_comments() {
        let src = r#"const MESSAGES = {
  // a comment
  en: { a: "x\"y", b: "line\nnext", c: 'single', d: "é" /* trailing */ },
};"#;
        let table = parse(src).unwrap();
        assert_eq!(table["en"]["a"], "x\"y");
        assert_eq!(table["en"]["b"], "line\nnext");
        assert_eq!(table["en"]["c"], "single");
        assert_eq!(table["en"]["d"], "é");
    }
}
