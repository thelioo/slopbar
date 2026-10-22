// The quick switcher (Ctrl+Alt+S by default, or the flyout's switch button): type part of an
// email, nickname or provider, move with the arrows, Enter switches, Esc closes.
import { t, translateDom, setLanguage, setSystemLocale, errorText, windowLabel } from "./i18n.js";
import { MARK } from "./marks.js";
import { observeFrontend } from "./diagnostics.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
observeFrontend("switcher");

const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };
const $query = document.getElementById("query");
const $results = document.getElementById("results");
const $hint = document.getElementById("switcher-hint");
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

let snapshot = { accounts: [] };
let shown = [];
let selected = 0;
let busy = false;

/** The binding window: the one closest to its limit, as the balancer compares. */
const peak = (a) => a.windows.reduce((m, w) => (w.used_percent > (m?.used_percent ?? -1) ? w : m), null);
const name = (a) => a.alias || a.email || t("unknownAccount");

/** Every word typed must appear in the account's name, email, plan or provider. */
export function matches(a, query) {
  const text = [a.alias, a.email, a.plan, a.org, PROVIDER_NAME[a.provider]].filter(Boolean).join(" ").toLowerCase();
  return query.toLowerCase().split(/\s+/).filter(Boolean).every((word) => text.includes(word));
}

/** Accounts with the most room first; the ones in use and unreadable ones after. */
function ranked(accounts, query) {
  const room = (a) => (a.stale || a.error || !peak(a) ? -1 : 100 - peak(a).used_percent);
  return accounts.filter((a) => matches(a, query))
    .sort((x, y) => Number(x.active) - Number(y.active) || room(y) - room(x) || name(x).localeCompare(name(y)));
}

function render() {
  shown = ranked(snapshot.accounts, $query.value);
  selected = Math.min(selected, Math.max(0, shown.length - 1));
  if (!shown.length) {
    $results.innerHTML = `<li class="empty">${esc(snapshot.accounts.length ? t("switcherNoMatch") : t("noAccounts"))}</li>`;
    return;
  }
  $results.innerHTML = shown.map((a, i) => {
    const w = a.stale || a.error ? null : peak(a);
    const room = w ? `<b>${Math.max(0, Math.round(100 - w.used_percent))}%</b><small>${esc(t("switcherLeft", { window: windowLabel(w) }))}</small>`
      : `<b>—</b><small>${esc(t("usageUnknown"))}</small>`;
    const meta = [PROVIDER_NAME[a.provider], a.alias ? a.email : "", a.plan].filter(Boolean).join(" · ");
    return `<li role="option" data-i="${i}" aria-selected="${i === selected}">
      <span class="mark-box" title="${PROVIDER_NAME[a.provider]}">${MARK[a.provider]}</span>
      <span class="who"><b>${esc(name(a))}</b><small>${esc(meta)}</small></span>
      <span class="room">${a.active ? `<span class="in-use">${t("inUse")}</span><br>` : ""}${room}</span>
    </li>`;
  }).join("");
  $results.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
}

async function choose(i) {
  const a = shown[i];
  if (!a || busy) return;
  if (a.active) { invoke("hide_switcher"); return; }
  busy = true;
  $hint.textContent = t("switching");
  try {
    snapshot = await invoke("switch_account", { provider: a.provider, id: a.id });
    invoke("hide_switcher");
  } catch (err) {
    $hint.textContent = errorText(PROVIDER_NAME[a.provider], String(err));
    $hint.classList.add("error");
  } finally {
    busy = false;
  }
}

$query.addEventListener("input", () => { selected = 0; render(); });
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") { e.preventDefault(); invoke("hide_switcher"); }
  else if (e.key === "ArrowDown") { e.preventDefault(); selected = Math.min(selected + 1, shown.length - 1); render(); }
  else if (e.key === "ArrowUp") { e.preventDefault(); selected = Math.max(selected - 1, 0); render(); }
  else if (e.key === "Enter") { e.preventDefault(); choose(selected); }
});
$results.addEventListener("click", (e) => {
  const row = e.target.closest("[data-i]");
  if (row) choose(Number(row.dataset.i));
});

function reset() {
  $query.value = "";
  selected = 0;
  $hint.textContent = t("switcherHint");
  $hint.classList.remove("error");
  render();
  $query.focus();
}

listen("switcher-open", async () => {
  reset();
  snapshot = await invoke("get_usage");
  render();
});
listen("usage-updated", (e) => { snapshot = e.payload; render(); });

function applySettings(settings) {
  setLanguage(settings.language);
  translateDom();
  $query.placeholder = t("switcherPlaceholder");
  $hint.textContent = t("switcherHint");
  render();
}
listen("settings-changed", (e) => applySettings(e.payload));
invoke("system_locale").then(setSystemLocale, () => {}).finally(() => invoke("get_settings").then(applySettings));

const theme = () => invoke("taskbar_theme").then((v) => (document.documentElement.dataset.theme = v));
theme();
setInterval(theme, 10_000);
invoke("get_usage").then((s) => { snapshot = s; render(); });
