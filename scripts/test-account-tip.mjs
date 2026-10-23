// Synthetic checks of the flyout's account tooltip (src/accounttext.js) and sparkline scaling
// (src/sparkline.js). Plain node, no browser: `node scripts/test-account-tip.mjs`. Every account
// here is made up; nothing contacts a provider.
import assert from "node:assert/strict";

// i18n.js sets the page language on load.
globalThis.document ??= { documentElement: {} };
const { t, setLanguage, windowResetText } = await import("../src/i18n.js");
const { accountTip, accountTipLines, adviceText } = await import("../src/accounttext.js");
const { sparkline, sparklineText } = await import("../src/sparkline.js");
setLanguage("en");

let passed = 0;
const test = (name, fn) => {
  try { fn(); passed++; } catch (err) { console.error(`FAIL ${name}\n${err.stack}`); process.exitCode = 1; }
};

const HOUR = 3600000;
const now = Date.now();
const iso = (ms) => new Date(ms).toISOString();

// ---------------------------------------------------------------------------------------------
// Sparklines

const pathOf = (svg) => /class="spark-line" d="([^"]*)"/.exec(svg)?.[1] ?? "";
const ys = (d) => [...d.matchAll(/[ML][\d.]+ ([\d.]+)/g)].map((m) => Number(m[1]));

test("no readings draw nothing, keep their place and say so", () => {
  for (const points of [undefined, null, []]) {
    const html = sparkline(points, { now });
    assert.match(html, /class="spark spark-empty"/);
    assert.match(html, /width:56px;height:16px/);
    assert.ok(!html.includes("spark-base") && !html.includes("<svg"), "no flat baseline");
    assert.ok(html.includes(`aria-label="${t("sparkNone")}"`));
  }
});

test("readings older than 7 days count as none", () => {
  const html = sparkline([[now - 8 * 24 * HOUR, 40]], { now });
  assert.match(html, /spark-empty/);
});

test("a flat 0% week is drawn along the bottom, not as an empty line", () => {
  const pts = [0, 1, 2, 3].map((h) => [now - (3 - h) * HOUR, 0]);
  const svg = sparkline(pts, { now, height: 16 });
  const d = pathOf(svg);
  assert.ok(d.includes("L"), "the readings are joined");
  assert.deepEqual([...new Set(ys(d))], [14.5], "at 0%, inside the padding");
  assert.match(svg, /class="spark-dot ok"/);
  assert.ok(svg.includes(sparklineText(pts)));
});

test("percentages scale onto the height, clamped to 0–100", () => {
  const pts = [[now - 3 * HOUR, 100], [now - 2 * HOUR, 50], [now - HOUR, 150], [now, -5]];
  const d = pathOf(sparkline(pts, { now, height: 16 }));
  assert.deepEqual(ys(d), [1.5, 8, 1.5, 14.5]);
  // The last reading sits at the right edge.
  assert.match(sparkline(pts, { now, width: 56 }), /cx="54.5"/);
});

test("readings hours apart are a gap, not a slope", () => {
  const d = pathOf(sparkline([[now - 10 * HOUR, 10], [now - HOUR, 20]], { now }));
  assert.equal((d.match(/M/g) ?? []).length, 2);
});

test("the row's own tooltip can stand in for the sparkline's", () => {
  const pts = [[now - HOUR, 10], [now, 20]];
  assert.ok(sparkline(pts, { now }).includes("<title>"));
  assert.ok(!sparkline(pts, { now, tooltip: false }).includes("<title>"));
  assert.ok(!sparkline([], { now, tooltip: false }).includes("title="));
});

// ---------------------------------------------------------------------------------------------
// The tooltip

const weekly = { kind: "weekly", used_percent: 77, window_seconds: 604800, resets_at: iso(now + 52 * HOUR) };
const opus = { kind: "weekly_opus", used_percent: 40, window_seconds: 604800, resets_at: iso(now + 52 * HOUR) };
const account = {
  id: "fixture-1", provider: "claude", email: "max@example.invalid", alias: "Work", plan: "Max 20x", org: null,
  active: true, sources: ["Windows"], windows: [weekly, opus], credits: null, error: null, monitor_error: null,
  stale: false, fetched_at: iso(now - 3 * 60000), monitor_connected: true, used_outside: true, leased: "nightly build",
  runs_out_at: iso(now + 30 * HOUR),
  remote: [{ machine: "LAPTOP-2", where: ["WSL: Ubuntu"], seen_at: iso(now), live: true },
    { machine: "DESK-3", where: ["Windows"], seen_at: iso(now - 2 * HOUR), live: false }],
  resets: [{ kind: "weekly", count: 2, usable: true, title: null, expires_at: iso(now + 20 * 24 * HOUR), next_available_at: null }],
};
const sub = {
  provider: "claude", id: "fixture-1", plan_kind: "personal", monthly_cost: 200, cost_entered: false,
  renewal: { at: iso(now + 5 * 24 * HOUR), source: "provider" }, ends_at: null, cancel_after: null,
  assessment: { verdict: "unused", renew_wasteful: true }, idle_now: false, delinquent: false,
  this_week: { used_percent: 77, pace_percent: 140, resets_at: weekly.resets_at, runs_out_at: null },
  credits: null, extra_usage: null,
};

test("a full account says everything the full card does, grouped", () => {
  const groups = accountTipLines(account, { sub, trail: [[now - HOUR, 70], [now, 77]], creditAdvice: "Spend it now", now });
  assert.equal(groups.length, 5);
  const [who, usage, state, held, plan] = groups;
  assert.equal(who[0], "Work (max@example.invalid)");
  assert.equal(who[1], "Claude · Max 20x");
  assert.equal(who[2], `${t("inUse")}: Windows, LAPTOP-2 (WSL: Ubuntu)`);
  assert.ok(who.some((l) => l.startsWith("DESK-3:")), "machines that went quiet");
  assert.ok(who.includes(t("usedOutside")) && who.includes(t("leased", { purpose: "nightly build" })));
  // Each window, model-specific ones too, with its reset relative and absolute, as the full card words it.
  assert.equal(usage[0], `${t("weekly")}: 77% ${t("used")} · ${windowResetText(weekly, false, account.fetched_at)}`);
  assert.match(usage[0], /Resets in 2d 4h · /);
  assert.ok(usage[1].startsWith(`${t("weekly_opus")}: 40% ${t("used")} · `));
  assert.ok(usage.some((l) => l.startsWith(t("runsOut", { at: "" }).trim())));
  assert.ok(usage.includes(t("tipPace", { p: 140 })));
  assert.ok(usage.includes(sparklineText([[now - HOUR, 70], [now, 77]])));
  assert.deepEqual(state, [t("lastObserved", { t: "3m" }), t("tipMonitorOn")]);
  assert.ok(held.some((l) => l.includes(t("resetUsableNow"))) && held.includes("Spend it now"));
  assert.ok(plan[0].includes("$200") && plan[0].includes(t("subSourceProvider")), plan[0]);
  assert.ok(plan.includes(t("tipVerdict", { v: t("subVerdict_unused") })));
  assert.ok(plan.includes(adviceText(sub)) && adviceText(sub));
});

test("the tooltip is plain lines with one blank line between groups", () => {
  const tip = accountTip(account, { sub, now });
  assert.ok(!tip.includes("\n\n\n") && !tip.startsWith("\n") && !tip.endsWith("\n"));
  assert.equal(tip.split("\n\n").length, 5);
});

test("missing fields are left out, not shown empty", () => {
  const bare = { id: "fixture-2", provider: "codex", email: "solo@example.invalid", alias: null, plan: null, org: null,
    active: false, sources: [], windows: [], credits: null, error: null, stale: false, fetched_at: null,
    monitor_connected: false, resets: [] };
  assert.deepEqual(accountTipLines(bare, { now }), [["solo@example.invalid", "Codex"], [t("tipMonitorOff")]]);
});

test("errors are worded with their fix, and stale readings say so", () => {
  const failing = { ...account, error: "token_expired", stale: true, runs_out_at: iso(now + HOUR) };
  const lines = accountTipLines(failing, { now }).flat();
  assert.ok(lines.includes(t("token_expired", { provider: "Claude Code" })));
  assert.ok(lines.includes(`${t("lastObserved", { t: "3m" })} · ${t("lastReading")}`));
  assert.ok(!lines.some((l) => l.startsWith(t("runsOut", { at: "" }).trim())), "no forecast from a stale reading");
});

console.log(`${passed} account tooltip and sparkline checks passed`);
