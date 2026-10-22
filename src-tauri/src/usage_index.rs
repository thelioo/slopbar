//! Tokens by account, day, repository and model, read from the CLIs' own files on this PC:
//! Claude Code's transcripts (`~/.claude/projects/**.jsonl`) and the Codex CLI's rollouts
//! (`~/.codex/sessions`, `archived_sessions`).
//!
//! Only model names, token counts, times and working folders are read; never prompts, replies or
//! tool output. Repository names stay on this PC: the Usage page, the agents' door and CSV
//! exports are their only readers.
//!
//! A Codex rollout names the account that started it (`creator_account_id`). A Claude transcript
//! names none, so its messages go to the account SlopBar saw signed in to Claude Code at the time
//! (`active_log`): that attribution is inferred, and a message from a time SlopBar was not
//! watching stays unattributed. Usage elsewhere (WSL, other machines, the web) is not here.
//!
//! Each file's counts are kept per minute, model and folder, in memory and in `usage-index.json`,
//! until its size or time changes; a transcript that only grew is read on from where it ended.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::active_log;

/// Files and minutes older than this are not read or kept.
pub const LOOKBACK_DAYS: i64 = 63;
const STORE_VERSION: u8 = 1;
const MAX_STORE: u64 = 96 << 20;
/// The store is rewritten at most this often once built (a restart re-reads only what grew).
const STORE_EVERY_SECONDS: i64 = 30 * 60;
/// Groups listed per slice; the rest are summed into one "other" group.
const TOP: usize = 25;
pub const OTHER: &str = "(other)";

// ---------------------------------------------------------------------------------------------
// Counts

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Input read fresh (not from a prompt cache).
    pub input: u64,
    /// Input written to the prompt cache (Claude); billed like input.
    pub cache_write: u64,
    /// Input read from the prompt cache.
    pub cached: u64,
    /// Output, reasoning included.
    pub output: u64,
    pub total: u64,
}

impl Tokens {
    fn new(input: u64, cache_write: u64, cached: u64, output: u64) -> Self {
        Tokens { input, cache_write, cached, output, total: input + cache_write + cached + output }
    }
    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cached += o.cached;
        self.output += o.output;
        self.total += o.total;
    }
}

/// One minute of one file: unix minute, model and folder (indexes into the file's lists), and
/// input, cache writes, cache reads and output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Row(i64, u16, u16, u64, u64, u64, u64);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct FileIndex {
    len: u64,
    mtime: i64,
    /// Bytes of whole lines read: a file that grew is read on from here.
    offset: u64,
    codex: bool,
    /// Codex: the account that started the session.
    account: Option<String>,
    models: Vec<String>,
    cwds: Vec<String>,
    rows: Vec<Row>,
    // Where the parser was, to continue after `offset`.
    header: bool,
    last_id: Option<String>,
    model: Option<u16>,
    cwd: Option<u16>,
    total: u64,
}

impl FileIndex {
    fn intern(list: &mut Vec<String>, s: &str) -> Option<u16> {
        if let Some(i) = list.iter().position(|x| x == s) { return Some(i as u16); }
        if list.len() >= u16::MAX as usize { return None; }
        list.push(s.chars().take(512).collect());
        Some((list.len() - 1) as u16)
    }

    fn add(&mut self, at: DateTime<Utc>, model: u16, cwd: u16, t: Tokens) {
        if t.total == 0 { return; }
        let minute = at.timestamp().div_euclid(60);
        if let Some(r) = self.rows.last_mut().filter(|r| r.0 == minute && r.1 == model && r.2 == cwd) {
            r.3 += t.input; r.4 += t.cache_write; r.5 += t.cached; r.6 += t.output;
            return;
        }
        self.rows.push(Row(minute, model, cwd, t.input, t.cache_write, t.cached, t.output));
    }

    fn line(&mut self, line: &[u8]) {
        if self.codex { self.codex_line(line) } else { self.claude_line(line) }
    }

    fn claude_line(&mut self, line: &[u8]) {
        #[derive(Deserialize)]
        struct Line { #[serde(rename = "type")] kind: Option<String>, timestamp: Option<String>, cwd: Option<String>, message: Option<Message> }
        #[derive(Deserialize)]
        struct Message { id: Option<String>, model: Option<String>, usage: Option<Usage> }
        #[derive(Deserialize)]
        struct Usage { input_tokens: Option<u64>, output_tokens: Option<u64>, cache_read_input_tokens: Option<u64>, cache_creation_input_tokens: Option<u64> }
        if !contains(line, b"\"assistant\"") || !contains(line, b"\"usage\"") { return; }
        let Ok(l) = serde_json::from_slice::<Line>(line) else { return };
        let (Some("assistant"), Some(m)) = (l.kind.as_deref(), l.message) else { return };
        let (Some(u), Some(model)) = (m.usage, m.model.filter(|m| !m.is_empty() && m != "<synthetic>")) else { return };
        let Some(at) = l.timestamp.as_deref().and_then(time) else { return };
        // A reply is written once per content block, each line repeating its usage.
        if m.id.is_some() && m.id == self.last_id { return; }
        self.last_id = m.id;
        let t = Tokens::new(u.input_tokens.unwrap_or(0), u.cache_creation_input_tokens.unwrap_or(0), u.cache_read_input_tokens.unwrap_or(0), u.output_tokens.unwrap_or(0));
        let (Some(model), Some(cwd)) = (Self::intern(&mut self.models, &model), Self::intern(&mut self.cwds, l.cwd.as_deref().unwrap_or(""))) else { return };
        self.add(at, model, cwd, t);
    }

    fn codex_line(&mut self, line: &[u8]) {
        #[derive(Deserialize)]
        struct Line { #[serde(rename = "type")] kind: Option<String>, timestamp: Option<String>, payload: Option<Payload> }
        #[derive(Deserialize)]
        struct Payload { #[serde(rename = "type")] kind: Option<String>, cwd: Option<String>, model: Option<String>, creator_account_id: Option<String>, info: Option<Info> }
        #[derive(Deserialize)]
        struct Info { total_token_usage: Option<Usage>, last_token_usage: Option<Usage> }
        #[derive(Deserialize)]
        struct Usage { input_tokens: Option<u64>, cached_input_tokens: Option<u64>, cache_write_input_tokens: Option<u64>, output_tokens: Option<u64>, total_tokens: Option<u64> }
        let header = !self.header && contains(line, b"\"session_meta\"");
        if !header && !contains(line, b"\"turn_context\"") && !contains(line, b"\"token_count\"") { return; }
        let Ok(l) = serde_json::from_slice::<Line>(line) else { return };
        let Some(p) = l.payload else { return };
        match (l.kind.as_deref(), p.kind.as_deref()) {
            (Some("session_meta"), _) if !self.header => {
                self.header = true;
                self.account = p.creator_account_id.filter(|s| !s.is_empty());
                if let Some(cwd) = p.cwd { self.cwd = Self::intern(&mut self.cwds, &cwd); }
            }
            (Some("turn_context"), _) => {
                if let Some(cwd) = p.cwd { self.cwd = Self::intern(&mut self.cwds, &cwd); }
                if let Some(model) = p.model.filter(|m| !m.is_empty()) { self.model = Self::intern(&mut self.models, &model); }
            }
            (Some("event_msg"), Some("token_count")) => {
                let Some(info) = p.info else { return };
                let Some(at) = l.timestamp.as_deref().and_then(time) else { return };
                // The same totals are repeated when only the rate limits changed.
                if let Some(total) = info.total_token_usage.as_ref().and_then(|u| u.total_tokens) {
                    if total == self.total { return; }
                    self.total = total;
                }
                let Some(u) = info.last_token_usage else { return };
                let (input, cached, write) = (u.input_tokens.unwrap_or(0), u.cached_input_tokens.unwrap_or(0), u.cache_write_input_tokens.unwrap_or(0));
                // Codex counts cached input inside input_tokens.
                let t = Tokens::new(input.saturating_sub(cached).saturating_sub(write), write, cached, u.output_tokens.unwrap_or(0));
                let model = match self.model { Some(m) => m, None => match Self::intern(&mut self.models, "unknown") { Some(m) => m, None => return } };
                let cwd = match self.cwd { Some(c) => c, None => match Self::intern(&mut self.cwds, "") { Some(c) => c, None => return } };
                self.add(at, model, cwd, t);
            }
            _ => {}
        }
    }
}

fn time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

/// Reads `path` on from `prev` when it only grew, else from the start. A last line without its
/// newline is still being written: it is read next time.
fn index_file(path: &Path, codex: bool, prev: Option<FileIndex>, len: u64, mtime: i64) -> (FileIndex, u64) {
    let zst = path.extension().is_some_and(|e| e == "zst");
    let mut fi = match prev {
        Some(p) if !zst && p.offset > 0 && p.len <= len && p.offset <= len => p,
        _ => FileIndex { codex, ..Default::default() },
    };
    let start = fi.offset;
    let Ok(mut file) = std::fs::File::open(path) else { return (fi, 0) };
    let reader: Box<dyn Read> = if zst {
        match zstd::stream::read::Decoder::new(file) { Ok(d) => Box::new(d), Err(_) => return (fi, 0) }
    } else {
        if file.seek(SeekFrom::Start(start)).is_err() { return (fi, 0); }
        Box::new(file)
    };
    let mut reader = BufReader::with_capacity(1 << 16, reader);
    let mut line = Vec::new();
    let mut read = 0u64;
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) { Ok(0) | Err(_) => break, Ok(_) => {} }
        if line.last() != Some(&b'\n') && !zst { break; }
        read += line.len() as u64;
        fi.line(&line);
    }
    fi.offset = if zst { len } else { start + read };
    (fi.len, fi.mtime) = (len, mtime);
    (fi, read)
}

// ---------------------------------------------------------------------------------------------
// Scanning

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store { version: u8, files: HashMap<String, FileIndex> }

/// What a scan did, for tests and timing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats { pub files: usize, pub read: usize, pub bytes: u64 }

/// Where the CLIs keep their files.
#[derive(Debug, Clone, Default)]
pub struct Roots { pub claude: Vec<PathBuf>, pub codex: Vec<PathBuf> }

impl Roots {
    pub fn local() -> Self {
        let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".claude")));
        let codex = crate::codex_local::codex_home();
        Roots {
            claude: claude.map(|c| vec![c.join("projects")]).unwrap_or_default(),
            codex: codex.map(|c| vec![c.join("sessions"), c.join("archived_sessions")]).unwrap_or_default(),
        }
    }
}

pub struct Scanner {
    store: Store,
    path: Option<PathBuf>,
    written: Option<DateTime<Utc>>,
    dirty: bool,
    index: Option<Arc<Index>>,
}

impl Scanner {
    pub fn new(path: Option<PathBuf>) -> Self {
        let store = path.as_deref().map(load_store).unwrap_or_default();
        Scanner { store, path, written: None, dirty: false, index: None }
    }

    /// Reads what changed under `roots` and returns the index. Blocking.
    pub fn scan(&mut self, roots: &Roots, now: DateTime<Utc>) -> (Arc<Index>, ScanStats) {
        let cutoff = SystemTime::now() - std::time::Duration::from_secs(LOOKBACK_DAYS as u64 * 86400);
        let mut files = vec![];
        for r in &roots.claude { collect(r, 3, cutoff, false, &mut files); }
        for r in &roots.codex { collect(r, 4, cutoff, true, &mut files); }
        let mut stats = ScanStats { files: files.len(), ..Default::default() };
        let mut seen = HashSet::new();
        let mut changed = false;
        for (path, codex, len, mtime) in files {
            let key = path.to_string_lossy().into_owned();
            let fresh = self.store.files.get(&key).is_some_and(|f| f.len == len && f.mtime == mtime && f.codex == codex);
            if !fresh {
                let prev = self.store.files.remove(&key).filter(|f| f.codex == codex);
                let (fi, bytes) = index_file(&path, codex, prev, len, mtime);
                self.store.files.insert(key.clone(), fi);
                stats.read += 1;
                stats.bytes += bytes;
                changed = true;
            }
            seen.insert(key);
        }
        let before = self.store.files.len();
        self.store.files.retain(|k, _| seen.contains(k));
        changed |= self.store.files.len() != before;
        if changed || self.index.is_none() {
            let oldest = (now - Duration::days(LOOKBACK_DAYS)).timestamp().div_euclid(60);
            for f in self.store.files.values_mut() { f.rows.retain(|r| r.0 >= oldest); }
            self.index = Some(Arc::new(Index::from_store(&self.store)));
        }
        self.dirty |= changed;
        let due = self.written.map_or(true, |w| (now - w).num_seconds() >= STORE_EVERY_SECONDS);
        if let Some(path) = self.path.as_deref().filter(|_| self.dirty && due) {
            self.store.version = STORE_VERSION;
            if let Ok(bytes) = serde_json::to_vec(&self.store) {
                if crate::private_store::write(path, &bytes).is_ok() { self.dirty = false; }
            }
            self.written = Some(now);
        }
        (self.index.clone().unwrap(), stats)
    }
}

/// The app's scanner, its store in the app folder. Blocking: call from `spawn_blocking`.
pub fn scan(now: DateTime<Utc>) -> Arc<Index> {
    static SCANNER: OnceLock<Mutex<Scanner>> = OnceLock::new();
    let scanner = SCANNER.get_or_init(|| Mutex::new(Scanner::new(crate::settings::app_dir().map(|d| d.join("usage-index.json")))));
    scanner.lock().unwrap().scan(&Roots::local(), now).0
}

fn load_store(path: &Path) -> Store {
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_STORE) { return Store::default(); }
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Store>(&b).ok())
        .filter(|s| s.version == STORE_VERSION).unwrap_or_default()
}

fn collect(dir: &Path, depth: u8, cutoff: SystemTime, codex: bool, out: &mut Vec<(PathBuf, bool, u64, i64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let path = e.path();
        if meta.is_dir() {
            if depth > 0 { collect(&path, depth - 1, cutoff, codex, out); }
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        let wanted = if codex { name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) } else { name.ends_with(".jsonl") };
        if !wanted { continue; }
        let Ok(mtime) = meta.modified() else { continue };
        if mtime < cutoff { continue; }
        let millis = mtime.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        out.push((path, codex, meta.len(), millis));
    }
}

// ---------------------------------------------------------------------------------------------
// The index and its questions

struct Source { codex: bool, account: Option<String>, models: Vec<String>, repos: Vec<String>, rows: Vec<Row> }

/// Every file's minutes, folders already named by repository.
#[derive(Default)]
pub struct Index { sources: Vec<Source> }

impl Index {
    fn from_store(store: &Store) -> Self {
        // Folders keep their repository between scans; a scan runs after every refresh.
        static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        let mut names = NAMES.get_or_init(Mutex::default).lock().unwrap();
        let sources = store.files.values().filter(|f| !f.rows.is_empty()).map(|f| Source {
            codex: f.codex,
            account: f.account.clone(),
            models: f.models.clone(),
            repos: f.cwds.iter().map(|c| names.entry(c.clone()).or_insert_with(|| repo_name(c)).clone()).collect(),
            rows: f.rows.clone(),
        }).collect();
        Index { sources }
    }
}

/// The repository a working folder belongs to: the git top level's folder name (a worktree's
/// main repository), else the folder's own name. Folders that no longer exist are walked up to
/// the nearest one that does, so a removed worktree still lands in its repository.
pub fn repo_name(cwd: &str) -> String {
    let leaf = || cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("(none)").to_string();
    let path = Path::new(cwd);
    if cwd.is_empty() { return "(none)".into(); }
    if !path.is_absolute() { return leaf(); }
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    for dir in path.ancestors() {
        let git = dir.join(".git");
        let Ok(meta) = std::fs::metadata(&git) else { continue };
        if meta.is_dir() { return name(dir).unwrap_or_else(leaf); }
        // A worktree's `.git` file points into the main repository's `.git/worktrees/<name>`.
        let main = std::fs::read_to_string(&git).ok()
            .and_then(|s| s.lines().next().and_then(|l| l.strip_prefix("gitdir:")).map(|g| PathBuf::from(g.trim())))
            .and_then(|g| g.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git")).and_then(Path::parent).and_then(name));
        return main.or_else(|| name(dir)).unwrap_or_else(leaf);
    }
    leaf()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy { #[default] Repo, Model, Day }

/// How a slice's tokens were tied to its account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    /// The session names its account (Codex).
    Session,
    /// The account SlopBar saw signed in to the CLI at the time (Claude, and Codex sessions that
    /// name none).
    InferredFromActiveAccount,
    /// Some of each.
    Mixed,
    /// From a time SlopBar did not see which account was signed in.
    Unattributed,
}

/// An account as the snapshot knows it.
#[derive(Debug, Clone)]
pub struct Known { pub provider: Provider, pub id: String, pub email: Option<String>, pub alias: Option<String> }

impl Known {
    pub fn from_snapshot(s: &crate::model::Snapshot) -> Vec<Known> {
        s.accounts.iter().map(|a| Known { provider: a.provider, id: a.id.clone(), email: a.email.clone(), alias: a.alias.clone() }).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Filter { pub provider: Option<Provider>, pub account: Option<String>, pub days: u32, pub group_by: GroupBy }

#[derive(Debug, Clone, Serialize)]
pub struct Group { pub key: String, pub tokens: Tokens, pub share: f64 }

/// Tokens by local weekday (Monday first) and hour.
pub type Heatmap = [[u64; 24]; 7];

#[derive(Debug, Clone, Serialize)]
pub struct Slice {
    pub provider: Provider,
    /// The slot id; none for the unattributed slice. A Codex account SlopBar does not know keeps
    /// its ChatGPT account id here, with `known` false.
    pub id: Option<String>,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub known: bool,
    pub attribution: Attribution,
    pub tokens: Tokens,
    /// Of `tokens.total`, how much was inferred from the active account.
    pub inferred_total: u64,
    pub groups: Vec<Group>,
    pub heatmap: Heatmap,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsage {
    pub provider: Provider,
    pub tokens: Tokens,
    pub unattributed: Tokens,
    /// Model mix across every account and the unattributed time.
    pub models: Vec<Group>,
    /// By the requested grouping, across the provider.
    pub groups: Vec<Group>,
    pub heatmap: Heatmap,
}

#[derive(Debug, Clone, Serialize)]
pub struct Breakdown {
    pub generated_at: DateTime<Utc>,
    pub days: u32,
    /// The first local day counted.
    pub since: NaiveDate,
    pub group_by: GroupBy,
    /// The local time zone's offset now, which days and heatmap hours follow.
    pub utc_offset_minutes: i32,
    pub providers: Vec<ProviderUsage>,
    /// Each known account of the providers asked for (with or without use), then any unknown
    /// Codex accounts and one unattributed slice per provider that has some.
    pub accounts: Vec<Slice>,
}

/// One minute's tokens, resolved to account, local time, repository and model.
struct Resolved<'a> { provider: Provider, slot: Option<SlotKey>, inferred: bool, local: NaiveDateTime, repo: &'a str, model: &'a str, tokens: Tokens }

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SlotKey { Known(String), Unknown(String) }

/// Walks every minute from `since` (UTC) on, resolving accounts.
fn resolve<'a, Tz: TimeZone>(index: &'a Index, log: &active_log::Log, known: &[Known], since: DateTime<Utc>, tz: &Tz, mut f: impl FnMut(Resolved<'a>)) {
    let codex_slots: HashMap<&str, &str> = known.iter().filter(|k| k.provider == Provider::Codex)
        .filter_map(|k| crate::codex_local::account_id(&k.id).map(|a| (a, k.id.as_str()))).collect();
    let first = since.timestamp().div_euclid(60);
    for s in &index.sources {
        let provider = if s.codex { Provider::Codex } else { Provider::Claude };
        let named = s.account.as_deref().map(|a| codex_slots.get(a).map_or_else(|| SlotKey::Unknown(a.to_string()), |id| SlotKey::Known(id.to_string())));
        for r in s.rows.iter().filter(|r| r.0 >= first) {
            let Some(at) = Utc.timestamp_opt(r.0 * 60, 0).single() else { continue };
            let (slot, inferred) = match &named {
                Some(k) => (Some(k.clone()), false),
                // A minute that starts before the account's first reading may still end inside it.
                None => (log.at(provider, at).or_else(|| log.at(provider, at + Duration::seconds(59))).map(|id| SlotKey::Known(id.to_string())), true),
            };
            f(Resolved {
                provider, inferred: inferred && slot.is_some(), slot,
                local: at.with_timezone(tz).naive_local(),
                repo: s.repos.get(r.2 as usize).map_or("(none)", String::as_str),
                model: s.models.get(r.1 as usize).map_or("unknown", String::as_str),
                tokens: Tokens::new(r.3, r.4, r.5, r.6),
            });
        }
    }
}

#[derive(Default)]
struct Acc { tokens: Tokens, inferred: u64, session: u64, groups: HashMap<String, Tokens>, models: HashMap<String, Tokens>, heatmap: Heatmap }

impl Acc {
    fn add(&mut self, r: &Resolved, key: &str) {
        self.tokens.add(&r.tokens);
        if r.inferred { self.inferred += r.tokens.total } else if r.slot.is_some() { self.session += r.tokens.total }
        self.groups.entry(key.to_string()).or_default().add(&r.tokens);
        self.models.entry(r.model.to_string()).or_default().add(&r.tokens);
        self.heatmap[r.local.weekday().num_days_from_monday() as usize][r.local.hour() as usize] += r.tokens.total;
    }
}

fn top(map: HashMap<String, Tokens>, total: u64, by_key: bool) -> Vec<Group> {
    let mut list: Vec<(String, Tokens)> = map.into_iter().collect();
    if by_key { list.sort_by(|a, b| b.0.cmp(&a.0)); } else { list.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.0.cmp(&b.0))); }
    let share = |t: &Tokens| if total == 0 { 0.0 } else { (t.total as f64 / total as f64 * 1000.0).round() / 10.0 };
    let mut out: Vec<Group> = vec![];
    let mut rest = Tokens::default();
    for (i, (key, tokens)) in list.into_iter().enumerate() {
        if i < TOP || by_key { out.push(Group { share: share(&tokens), key, tokens }); } else { rest.add(&tokens); }
    }
    if rest.total > 0 { out.push(Group { key: OTHER.into(), share: share(&rest), tokens: rest }); }
    out
}

/// Tokens over the last `filter.days` local days (today included).
pub fn breakdown<Tz: TimeZone>(index: &Index, log: &active_log::Log, known: &[Known], filter: &Filter, now: DateTime<Utc>, tz: &Tz) -> Breakdown {
    let days = filter.days.clamp(1, LOOKBACK_DAYS as u32);
    let today = now.with_timezone(tz).date_naive();
    let since = today - Duration::days(days as i64 - 1);
    let start = tz.from_local_datetime(&since.and_hms_opt(0, 0, 0).unwrap()).earliest().map_or(now - Duration::days(days as i64), |t| t.with_timezone(&Utc));
    let wanted = |p: Provider| filter.provider.map_or(true, |f| f == p);
    let mut slices: BTreeMap<(u8, Option<SlotKey>), Acc> = BTreeMap::new();
    let mut providers: BTreeMap<u8, Acc> = BTreeMap::new();
    let order = |p: Provider| if p == Provider::Claude { 0u8 } else { 1 };
    resolve(index, log, known, start, tz, |r| {
        if !wanted(r.provider) || r.local.date() < since { return; }
        if let Some(a) = &filter.account {
            if !matches!(&r.slot, Some(SlotKey::Known(id) | SlotKey::Unknown(id)) if id == a) { return; }
        }
        let key = match filter.group_by {
            GroupBy::Repo => r.repo.to_string(),
            GroupBy::Model => r.model.to_string(),
            GroupBy::Day => r.local.date().to_string(),
        };
        providers.entry(order(r.provider)).or_default().add(&r, &key);
        slices.entry((order(r.provider), r.slot.clone())).or_default().add(&r, &key);
    });
    let by_day = filter.group_by == GroupBy::Day;
    let provider_of = |o: u8| if o == 0 { Provider::Claude } else { Provider::Codex };
    let mut accounts = vec![];
    for k in known.iter().filter(|k| wanted(k.provider) && filter.account.as_ref().map_or(true, |a| *a == k.id)) {
        let acc = slices.remove(&(order(k.provider), Some(SlotKey::Known(k.id.clone())))).unwrap_or_default();
        accounts.push(slice(k.provider, Some(k.id.clone()), k.email.clone(), k.alias.clone(), true, acc, by_day));
    }
    for ((o, key), acc) in slices {
        match key {
            // An account seen in the active log that the snapshot no longer lists.
            Some(SlotKey::Known(id)) => accounts.push(slice(provider_of(o), Some(id), None, None, false, acc, by_day)),
            Some(SlotKey::Unknown(id)) => accounts.push(slice(provider_of(o), Some(id), None, None, false, acc, by_day)),
            None => accounts.push(slice(provider_of(o), None, None, None, false, acc, by_day)),
        }
    }
    let unattributed: HashMap<Provider, Tokens> = accounts.iter().filter(|s| s.id.is_none()).map(|s| (s.provider, s.tokens)).collect();
    let providers = Provider::ALL.into_iter().filter(|p| wanted(*p)).map(|p| {
        let acc = providers.remove(&order(p)).unwrap_or_default();
        ProviderUsage {
            provider: p,
            tokens: acc.tokens,
            unattributed: unattributed.get(&p).copied().unwrap_or_default(),
            models: top(acc.models, acc.tokens.total, false),
            groups: top(acc.groups, acc.tokens.total, by_day),
            heatmap: acc.heatmap,
        }
    }).collect();
    let utc_offset_minutes = now.with_timezone(tz).offset().fix().local_minus_utc() / 60;
    Breakdown { generated_at: now, days, since, group_by: filter.group_by, utc_offset_minutes, providers, accounts }
}

fn slice(provider: Provider, id: Option<String>, email: Option<String>, alias: Option<String>, known: bool, acc: Acc, by_day: bool) -> Slice {
    let attribution = match (id.is_some(), acc.session > 0, acc.inferred > 0) {
        (false, _, _) => Attribution::Unattributed,
        (true, true, true) => Attribution::Mixed,
        (true, _, true) => Attribution::InferredFromActiveAccount,
        (true, _, false) if provider == Provider::Claude => Attribution::InferredFromActiveAccount,
        _ => Attribution::Session,
    };
    Slice { provider, id, email, alias, known, attribution, tokens: acc.tokens, inferred_total: acc.inferred,
        groups: top(acc.groups, acc.tokens.total, by_day), heatmap: acc.heatmap }
}

/// One CSV line of the breakdown: tokens per local day, account, repository and model.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyRow {
    pub day: NaiveDate,
    pub provider: Provider,
    pub id: Option<String>,
    pub email: Option<String>,
    pub attribution: Attribution,
    pub repo: String,
    pub model: String,
    pub tokens: Tokens,
}

/// Every local day of the lookback, oldest first.
pub fn daily<Tz: TimeZone>(index: &Index, log: &active_log::Log, known: &[Known], now: DateTime<Utc>, tz: &Tz) -> Vec<DailyRow> {
    let emails: HashMap<&str, Option<&String>> = known.iter().map(|k| (k.id.as_str(), k.email.as_ref())).collect();
    let mut rows: BTreeMap<(NaiveDate, u8, Option<SlotKey>, bool, String, String), Tokens> = BTreeMap::new();
    resolve(index, log, known, now - Duration::days(LOOKBACK_DAYS), tz, |r| {
        let p = if r.provider == Provider::Claude { 0 } else { 1 };
        rows.entry((r.local.date(), p, r.slot.clone(), r.inferred, r.repo.to_string(), r.model.to_string())).or_default().add(&r.tokens);
    });
    rows.into_iter().map(|((day, p, slot, inferred, repo, model), tokens)| {
        let id = slot.map(|s| match s { SlotKey::Known(id) | SlotKey::Unknown(id) => id });
        DailyRow {
            day, provider: if p == 0 { Provider::Claude } else { Provider::Codex },
            email: id.as_deref().and_then(|i| emails.get(i).copied().flatten().cloned()),
            attribution: match (&id, inferred) { (None, _) => Attribution::Unattributed, (Some(_), true) => Attribution::InferredFromActiveAccount, _ => Attribution::Session },
            id, repo, model, tokens,
        }
    }).collect()
}

/// One repository and model of one account on this PC: its tokens in the last window, and its
/// usual tokens per window of that length in the time before, counting only windows it was used in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalRate {
    pub provider: Provider,
    /// The slot id (or a Codex account id SlopBar does not know); None when unattributed.
    pub id: Option<String>,
    pub repo: String,
    pub model: String,
    pub recent: u64,
    pub baseline: Option<f64>,
    /// Windows of the baseline it was used in.
    pub active_windows: u32,
}

/// Every repository and model used in the last `window` or the `baseline` before it, busiest
/// recently first.
pub fn local_rates(index: &Index, log: &active_log::Log, known: &[Known], window: Duration, baseline: Duration, now: DateTime<Utc>) -> Vec<LocalRate> {
    let split = now - window;
    let mut acc: HashMap<(Provider, Option<String>, String, String), (u64, HashMap<i64, u64>)> = HashMap::new();
    let width = window.num_seconds().max(60);
    resolve(index, log, known, split - baseline, &Utc, |r| {
        let at = r.local.and_utc();
        if at > now { return; }
        let id = r.slot.as_ref().map(|s| match s { SlotKey::Known(id) | SlotKey::Unknown(id) => id.clone() });
        let entry = acc.entry((r.provider, id, r.repo.to_string(), r.model.to_string())).or_default();
        if at >= split { entry.0 += r.tokens.total; } else { *entry.1.entry((split - at).num_seconds() / width).or_default() += r.tokens.total; }
    });
    let mut out: Vec<LocalRate> = acc.into_iter().map(|((provider, id, repo, model), (recent, buckets))| {
        let active = buckets.values().filter(|t| **t > 0).count() as u32;
        LocalRate { provider, id, repo, model, recent, active_windows: active,
            baseline: (active > 0).then(|| buckets.values().sum::<u64>() as f64 / active as f64) }
    }).collect();
    out.sort_by(|a, b| b.recent.cmp(&a.recent).then_with(|| (&a.repo, &a.model).cmp(&(&b.repo, &b.model))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::io::Write;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    /// An assistant line shaped like Claude Code's, its content a stand-in that must never be read.
    fn claude(id: &str, ts: &str, cwd: &str, model: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> String {
        format!("{}\n", serde_json::json!({ "type": "assistant", "timestamp": ts, "cwd": cwd, "sessionId": "s1",
            "message": { "id": id, "model": model, "role": "assistant", "content": [{ "type": "text", "text": "SECRET reply text" }],
                "usage": { "input_tokens": input, "output_tokens": output, "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": cache_write } } }))
    }

    fn user(ts: &str) -> String {
        format!("{}\n", serde_json::json!({ "type": "user", "timestamp": ts, "cwd": "C:\\x", "message": { "role": "user", "content": "SECRET prompt \"usage\" assistant" } }))
    }

    fn codex_lines(account: Option<&str>, cwd: &str) -> Vec<String> {
        let mut meta = serde_json::json!({ "timestamp": "2026-10-20T09:00:00Z", "type": "session_meta", "payload": { "id": "t1", "cwd": cwd, "base_instructions": "SECRET" } });
        if let Some(a) = account { meta["payload"]["creator_account_id"] = a.into(); }
        let turn = serde_json::json!({ "timestamp": "2026-10-20T09:00:01Z", "type": "turn_context", "payload": { "cwd": cwd, "model": "gpt-5.5-codex" } });
        let count = |ts: &str, total: u64, input: u64, cached: u64, output: u64| serde_json::json!({ "timestamp": ts, "type": "event_msg", "payload": { "type": "token_count",
            "info": { "total_token_usage": { "total_tokens": total }, "last_token_usage": { "input_tokens": input, "cached_input_tokens": cached, "output_tokens": output, "total_tokens": input + output } },
            "rate_limits": { "limit_id": "codex" } } });
        let message = serde_json::json!({ "timestamp": "2026-10-20T09:00:02Z", "type": "response_item", "payload": { "type": "message", "content": [{ "text": "SECRET token_count" }] } });
        [meta, turn, message, count("2026-10-20T09:01:00Z", 1100, 1000, 600, 100),
            // A repeat with the same totals (only the rate limits moved) adds nothing.
            count("2026-10-20T09:01:30Z", 1100, 1000, 600, 100),
            count("2026-10-20T09:05:00Z", 1350, 200, 0, 50)]
            .iter().map(|v| format!("{v}\n")).collect()
    }

    fn known() -> Vec<Known> {
        vec![
            Known { provider: Provider::Claude, id: "slot-a".into(), email: Some("a@example.invalid".into()), alias: None },
            Known { provider: Provider::Claude, id: "slot-b".into(), email: Some("b@example.invalid".into()), alias: None },
            Known { provider: Provider::Codex, id: "user-1:acct-1".into(), email: Some("c@example.invalid".into()), alias: None },
        ]
    }

    fn roots(dir: &Path) -> Roots { Roots { claude: vec![dir.join("claude")], codex: vec![dir.join("codex")] } }

    #[test]
    fn claude_switch_mid_session_and_gaps() {
        let dir = TempDir::new();
        let project = dir.0.join("claude").join("C--work-app");
        std::fs::create_dir_all(&project).unwrap();
        let repo = dir.0.join("app");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let cwd = repo.join("src").to_string_lossy().into_owned();
        let body = [
            user("2026-10-20T10:00:00Z"),
            claude("m1", "2026-10-20T10:01:10Z", &cwd, "claude-opus-5-5", 10, 20, 300, 40),
            // The same reply's second content block repeats its usage.
            claude("m1", "2026-10-20T10:01:11Z", &cwd, "claude-opus-5-5", 10, 20, 300, 40),
            claude("m2", "2026-10-20T10:30:00Z", &cwd, "claude-sonnet-5-5", 1, 2, 0, 0),
            claude("m3", "2026-10-20T13:00:00Z", &cwd, "claude-opus-5-5", 5, 5, 0, 0),
            claude("m4", "2026-10-20T10:20:00Z", &cwd, "<synthetic>", 0, 0, 0, 0),
        ].concat();
        std::fs::write(project.join("s1.jsonl"), body).unwrap();
        let gap = active_log::max_gap(5);
        let mut log = active_log::Log::default();
        for m in ["10:00", "10:05", "10:10", "10:15"] { log.note(Provider::Claude, "slot-a", at(&format!("2026-10-20T{m}:00Z")), gap); }
        // Switched to b mid-session; the app then closed at 10:35, so 13:00 is a gap.
        for m in ["10:25", "10:30", "10:35"] { log.note(Provider::Claude, "slot-b", at(&format!("2026-10-20T{m}:00Z")), gap); }
        let mut scanner = Scanner::new(None);
        let (index, _) = scanner.scan(&roots(&dir.0), at("2026-10-21T12:00:00Z"));
        let f = Filter { provider: Some(Provider::Claude), account: None, days: 7, group_by: GroupBy::Model };
        let b = breakdown(&index, &log, &known(), &f, at("2026-10-21T12:00:00Z"), &Utc);
        let a = b.accounts.iter().find(|s| s.id.as_deref() == Some("slot-a")).unwrap();
        assert_eq!(a.tokens, Tokens { input: 10, cache_write: 40, cached: 300, output: 20, total: 370 }, "m1 counted once");
        assert_eq!(a.attribution, Attribution::InferredFromActiveAccount);
        assert_eq!(a.inferred_total, 370);
        assert_eq!(a.groups[0].key, "claude-opus-5-5");
        let bb = b.accounts.iter().find(|s| s.id.as_deref() == Some("slot-b")).unwrap();
        assert_eq!(bb.tokens.total, 3);
        let none = b.accounts.iter().find(|s| s.id.is_none()).unwrap();
        assert_eq!((none.tokens.total, none.attribution), (10, Attribution::Unattributed));
        assert_eq!(b.providers[0].unattributed.total, 10);
        assert_eq!(b.providers[0].tokens.total, 383);
        let mix: Vec<_> = b.providers[0].models.iter().map(|g| (g.key.as_str(), g.share)).collect();
        assert_eq!(mix, [("claude-opus-5-5", 99.2), ("claude-sonnet-5-5", 0.8)]);
        // Tuesday 2026-10-20, 10:00 UTC.
        assert_eq!(a.heatmap[1][10], 370);
        let by_repo = breakdown(&index, &log, &known(), &Filter { group_by: GroupBy::Repo, ..f.clone() }, at("2026-10-21T12:00:00Z"), &Utc);
        assert_eq!(by_repo.providers[0].groups[0].key, "app", "the git top level, not the subfolder");
        // Outside the window nothing counts.
        let old = breakdown(&index, &log, &known(), &f, at("2026-11-30T12:00:00Z"), &Utc);
        assert_eq!(old.providers[0].tokens.total, 0);
        // Hours follow the local zone.
        let east = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        let shifted = breakdown(&index, &log, &known(), &f, at("2026-10-21T12:00:00Z"), &east);
        assert_eq!(shifted.accounts.iter().find(|s| s.id.as_deref() == Some("slot-a")).unwrap().heatmap[1][13], 370);
        assert_eq!(shifted.utc_offset_minutes, 180);
    }

    #[test]
    fn codex_sessions_name_their_account_and_repeats_count_once() {
        let dir = TempDir::new();
        let day = dir.0.join("codex").join("2026").join("10").join("20");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-a.jsonl"), codex_lines(Some("acct-1"), "/home/dev/tool").concat()).unwrap();
        let mut z = zstd::stream::write::Encoder::new(std::fs::File::create(day.join("rollout-b.jsonl.zst")).unwrap(), 0).unwrap();
        z.write_all(codex_lines(Some("acct-9"), "D:\\gone\\other").concat().as_bytes()).unwrap();
        z.finish().unwrap();
        std::fs::write(day.join("rollout-c.jsonl"), codex_lines(None, "/home/dev/tool").concat()).unwrap();
        std::fs::write(day.join("notes.jsonl"), codex_lines(Some("acct-1"), "/x").concat()).unwrap();
        let mut log = active_log::Log::default();
        log.note(Provider::Codex, "user-1:acct-1", at("2026-10-20T08:58:00Z"), active_log::max_gap(5));
        log.note(Provider::Codex, "user-1:acct-1", at("2026-10-20T09:10:00Z"), active_log::max_gap(5));
        let (index, stats) = Scanner::new(None).scan(&roots(&dir.0), at("2026-10-21T00:00:00Z"));
        assert_eq!(stats.files, 3, "only rollouts");
        let f = Filter { provider: Some(Provider::Codex), account: None, days: 28, group_by: GroupBy::Repo };
        let b = breakdown(&index, &log, &known(), &f, at("2026-10-21T00:00:00Z"), &Utc);
        let one = b.accounts.iter().find(|s| s.id.as_deref() == Some("user-1:acct-1")).unwrap();
        // 400 fresh + 600 cached + 100 out, then 200 + 50; twice (named, and inferred for c).
        assert_eq!(one.tokens, Tokens { input: 1200, cache_write: 0, cached: 1200, output: 300, total: 2700 });
        assert_eq!(one.attribution, Attribution::Mixed);
        assert_eq!(one.inferred_total, 1350);
        assert_eq!(one.groups[0].key, "tool", "a folder outside git keeps its own name");
        let unknown = b.accounts.iter().find(|s| s.id.as_deref() == Some("acct-9")).unwrap();
        assert!(!unknown.known);
        assert_eq!((unknown.tokens.total, unknown.attribution), (1350, Attribution::Session));
        assert_eq!(b.providers[0].models[0].key, "gpt-5.5-codex");
        assert!(b.accounts.iter().all(|s| s.provider == Provider::Codex));
        let only = breakdown(&index, &log, &known(), &Filter { account: Some("acct-9".into()), ..f }, at("2026-10-21T00:00:00Z"), &Utc);
        assert_eq!(only.providers[0].tokens.total, 1350);
        let rows = daily(&index, &log, &known(), at("2026-10-21T00:00:00Z"), &Utc);
        assert_eq!(rows.iter().map(|r| r.tokens.total).sum::<u64>(), 4050);
        assert!(rows.iter().any(|r| r.email.as_deref() == Some("c@example.invalid") && r.attribution == Attribution::InferredFromActiveAccount));
    }

    #[test]
    fn grown_files_are_read_on_and_unchanged_ones_not_at_all() {
        let dir = TempDir::new();
        let project = dir.0.join("claude").join("p");
        std::fs::create_dir_all(&project).unwrap();
        let file = project.join("s.jsonl");
        let first = claude("m1", "2026-10-20T10:00:00Z", "/r", "claude-opus-5-5", 1, 1, 0, 0);
        let half = claude("m2", "2026-10-20T10:02:00Z", "/r", "claude-opus-5-5", 2, 2, 0, 0);
        // The second reply is still being written: no newline yet.
        std::fs::write(&file, format!("{first}{}", &half[..half.len() / 2])).unwrap();
        let store = dir.0.join("usage-index.json");
        let now = at("2026-10-21T00:00:00Z");
        let mut log = active_log::Log::default();
        log.note(Provider::Claude, "slot-a", at("2026-10-20T09:59:00Z"), active_log::max_gap(5));
        log.note(Provider::Claude, "slot-a", at("2026-10-20T10:05:00Z"), active_log::max_gap(5));
        let total = |index: &Index| breakdown(index, &log, &known(), &Filter { provider: None, account: Some("slot-a".into()), days: 7, group_by: GroupBy::Day }, now, &Utc).providers[0].tokens.total;
        let mut scanner = Scanner::new(Some(store.clone()));
        let (index, stats) = scanner.scan(&roots(&dir.0), now);
        assert_eq!((stats.read, total(&index)), (1, 2));
        let (_, stats) = scanner.scan(&roots(&dir.0), now);
        assert_eq!(stats.read, 0, "unchanged files are not read again");
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(&half.as_bytes()[half.len() / 2..]).unwrap();
        // Half an hour on, the store is due to be rewritten.
        let (index, stats) = scanner.scan(&roots(&dir.0), now + Duration::minutes(30));
        assert_eq!((stats.read, stats.bytes, total(&index)), (1, half.len() as u64, 6), "only the new line is read");
        // The store carries the work over a restart, without any reply text.
        assert!(store.exists());
        let saved = std::fs::read_to_string(&store).unwrap();
        assert!(!saved.contains("SECRET"));
        drop(scanner);
        let mut again = Scanner::new(Some(store.clone()));
        let (index, stats) = again.scan(&roots(&dir.0), now);
        assert_eq!((stats.read, total(&index)), (0, 6));
        let by_day = breakdown(&index, &log, &known(), &Filter { provider: Some(Provider::Claude), account: None, days: 7, group_by: GroupBy::Day }, now, &Utc);
        assert_eq!(by_day.providers[0].groups[0].key, "2026-10-20");
        // A rewritten (shorter) file is read from the start.
        std::fs::write(&file, &first).unwrap();
        let (index, _) = again.scan(&roots(&dir.0), now);
        assert_eq!(total(&index), 2);
        std::fs::remove_file(&file).unwrap();
        let (index, _) = again.scan(&roots(&dir.0), now);
        assert_eq!(total(&index), 0, "a deleted transcript leaves the index");
    }

    #[test]
    fn repositories_resolve_through_worktrees_and_missing_folders() {
        let dir = TempDir::new();
        let main = dir.0.join("slopper");
        std::fs::create_dir_all(main.join(".git").join("worktrees").join("wt1")).unwrap();
        let wt = dir.0.join("elsewhere").join("wt1");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", main.join(".git").join("worktrees").join("wt1").display())).unwrap();
        assert_eq!(repo_name(&wt.join("deep").to_string_lossy()), "slopper");
        assert_eq!(repo_name(&main.join(".claude").join("worktrees").join("removed").to_string_lossy()), "slopper");
        assert_eq!(repo_name(&dir.0.join("plain").join("folder").to_string_lossy()), "folder");
        assert_eq!(repo_name("/home/dev/project/"), "project");
        assert_eq!(repo_name(""), "(none)");
    }

    #[test]
    fn local_rates_compare_the_last_window_with_the_windows_used_before() {
        let dir = TempDir::new();
        let project = dir.0.join("claude").join("C--work-app");
        std::fs::create_dir_all(&project).unwrap();
        let cwd = dir.0.join("app").to_string_lossy().into_owned();
        let sent = [
            ("m1", "2026-10-20T10:00:00Z", 100), ("m2", "2026-10-19T10:00:00Z", 300), ("m4", "2026-10-21T10:59:00Z", 200),
            ("m3", "2026-10-21T11:30:00Z", 1000), ("m5", "2026-10-13T10:00:00Z", 5000), ("m6", "2026-10-21T12:30:00Z", 7),
        ];
        std::fs::write(project.join("s1.jsonl"), sent.iter().map(|(id, ts, n)| claude(id, ts, &cwd, "claude-opus-5-5", *n, 0, 0, 0)).collect::<String>()).unwrap();
        let mut log = active_log::Log::default();
        let mut times: Vec<_> = sent.iter().map(|(_, ts, _)| at(ts)).collect();
        times.sort();
        for t in times { for m in [-1, 1] { log.note(Provider::Claude, "slot-a", t + Duration::minutes(m), active_log::max_gap(5)); } }
        let now = at("2026-10-21T12:00:00Z");
        let (index, _) = Scanner::new(None).scan(&roots(&dir.0), now);
        let rates = local_rates(&index, &log, &known(), Duration::minutes(60), Duration::days(7), now);
        // The last hour has 1000; before it three hours of use (100, 300, 200), the one 8 days back
        // outside the baseline and the one after `now` nowhere.
        assert_eq!(rates, [LocalRate { provider: Provider::Claude, id: Some("slot-a".into()), repo: "app".into(), model: "claude-opus-5-5".into(),
            recent: 1000, baseline: Some(200.0), active_windows: 3 }]);
    }

    #[test]
    fn groups_beyond_the_top_are_summed() {
        let map: HashMap<String, Tokens> = (0..30).map(|i| (format!("r{i:02}"), Tokens::new(i + 1, 0, 0, 0))).collect();
        let total = (1..=30).sum();
        let groups = top(map, total, false);
        assert_eq!(groups.len(), TOP + 1);
        assert_eq!(groups[0].key, "r29");
        assert_eq!(groups.last().unwrap().key, OTHER);
        assert_eq!(groups.last().unwrap().tokens.total, (1..=5).sum::<u64>());
    }
}
