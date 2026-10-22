// Optional UI smoke check: install Playwright, then run `node scripts/test-telemetry-ui.cjs`.
// All Tauri commands and account data are synthetic. No provider requests or sign-ins occur.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { runResetScheduleMatrix } = require('./test-reset-schedule-matrix.cjs');

const root = path.resolve(__dirname, '../src');
const server = http.createServer((req, res) => {
  const file = path.resolve(root, '.' + req.url.split('?')[0]);
  if (!file.startsWith(root + path.sep) || !fs.existsSync(file)) { res.writeHead(404); res.end(); return; }
  res.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' })[path.extname(file)] || 'application/octet-stream');
  fs.createReadStream(file).pipe(res);
});

const data = {
  updated_at: new Date().toISOString(),
  accounts: [
    { id: 'inactive:org', provider: 'claude', email: 'inactive@example.invalid', plan: 'max', active: false, monitor_connected: false, stale: true, error: 'monitor_setup', percent: 87 },
    { id: 'connected:org', provider: 'claude', email: 'connected@example.invalid', plan: 'max', active: true, monitor_connected: true, stale: false, error: null, monitor_error: 'monitor_reconnect', percent: 24 },
    { id: 'reconnect:org', provider: 'codex', email: 'reconnect@example.invalid', plan: 'plus', active: true, monitor_connected: true, stale: true, error: 'monitor_reconnect', percent: 52 },
    { id: 'zero:org', provider: 'claude', email: 'zero@example.invalid', plan: 'max', active: false, monitor_connected: true, stale: false, error: null, percent: 0 },
  ].map(a => ({ ...a, alias: null, org: null, credits: null, resets: [], sources: a.active ? ['Windows'] : [],
    fetched_at: new Date(Date.now() - (a.stale ? 7200000 : 1000)).toISOString(),
    windows: [{ kind: 'session', used_percent: a.percent, resets_at: new Date(Date.now() - 3600000).toISOString(), window_seconds: 18000 }],
  })),
};
// Synthetic replay of the observed disagreement: one headline hides two
// windows; available reset counts do not identify their scope.
data.accounts[1].windows.push({ kind: 'weekly', used_percent: 58,
  resets_at: '2026-10-16T05:59:59Z', window_seconds: 604800 });
data.accounts[2].resets = [{ kind: 'unknown', title: 'Full reset', count: 2,
  usable: true, expires_at: '2026-10-29T19:00:11Z', next_available_at: null }];
// Reproduces a monitored idle account: the 0% session wins the headline tie,
// but only its separate weekly window has a provider-supplied reset time.
const idleWeeklyReset = new Date(Date.now() + 6 * 86400000).toISOString();
data.accounts[3].windows = [
  { kind: 'session', used_percent: 0, resets_at: null, window_seconds: 18000 },
  { kind: 'weekly', used_percent: 0, resets_at: idleWeeklyReset, window_seconds: 604800 },
];

async function setup(page, label) {
  await page.addInitScript(({ data, label }) => {
    window.testCalls = [];
    const listeners = {};
    window.testEmit = (name, payload) => (listeners[name] || []).forEach(fn => fn({ payload }));
    window.__TAURI__ = {
      core: { invoke: async (command, args) => {
        window.testCalls.push({ command, args });
        if (command === 'get_usage' || command === 'refresh') return data;
        if (command === 'disconnect_monitoring') return { snapshot: data, revoked: true };
        if (command === 'export_diagnostics') { if (window.testExportFail) throw new Error('PRIVATE-export-error'); return 'C:\\test\\SlopBar-Diagnostics.json'; }
        if (command === 'get_settings') return { providers: { claude: true, codex: true }, language: 'en', aliases: {}, refresh_minutes: 5, launch_at_login: false, auto_update: true,
          notifications: { usage: true, resets: true, switches: true }, balancer: { claude: { auto: false, threshold: 90 }, codex: { auto: false, threshold: 90 } } };
        if (command === 'system_locale') return 'en-US';
        if (command === 'taskbar_theme') return 'dark';
        if (command === 'app_version') return '0.1.0';
        if (command === 'has_mica' || command === 'updates_enabled') return false;
        return null;
      } },
      event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); return () => {}; } },
      window: { getCurrentWindow: () => ({ label, close: () => {} }) },
    };
  }, { data, label });
}

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true, ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) });
  const errors = [];
  try {
    const settings = await browser.newPage({ viewport: { width: 1000, height: 850 } });
    settings.on('pageerror', err => errors.push(String(err)));
    await setup(settings, 'settings');
    await settings.goto(base + '/settings.html');
    await settings.locator('[data-key="claude:inactive:org"] [data-monitor]').waitFor();
    assert.match(await settings.locator('#accounts').innerText(), /Last confirmed 2h ago/);
    assert.match(await settings.locator('[data-key="claude:inactive:org"]').innerText(), /87%/);
    assert.equal(await settings.locator('#auto-update').isDisabled(), true);
    assert.equal(await settings.locator('#auto-update').isChecked(), false);
    assert.match(await settings.locator('[data-key="claude:connected:org"]').innerText(), /Reconnect monitoring/);
    assert.match(await settings.locator('[data-key="claude:connected:org"] .meter').innerText(), /58%[\s\S]*Weekly · used/);
    assert.match(await settings.locator('[data-key="claude:connected:org"] .meter').getAttribute('title'), /Session \(5h\): 24% used[\s\S]*Weekly: 58% used/);
    assert.doesNotMatch(await settings.locator('#accounts').innerText(), /Model-specific usage unavailable from this source|Independent monitoring connected|Reset time unavailable/);
    const resetTooltip = await settings.locator('[data-key="codex:reconnect:org"] .resets').getAttribute('title');
    assert.match(resetTooltip, /type unknown[\s\S]*usable now/);
    assert.doesNotMatch(resetTooltip, /Full reset/);
    assert.equal(await settings.locator('[data-key="claude:connected:org"] .meter.stale').count(), 0);
    const idle = settings.locator('[data-key="claude:zero:org"]');
    assert.match(await idle.locator('.meter').innerText(), /0%[\s\S]*Weekly · used/);
    assert.match(await idle.locator('.window-resets').innerText(), /^Weekly: 0% used · Resets in/);
    assert.equal(await idle.locator('.window-reset').count(), 1);
    assert.equal(await idle.locator('.window-reset').last().getAttribute('title'), idleWeeklyReset);
    assert.equal(await settings.locator('[data-key="claude:inactive:org"] .window-reset').count(), 1);
    assert.match(await settings.locator('[data-key="claude:inactive:org"] .window-resets').innerText(), /Last known reset:/);
    assert.match(await settings.locator('[data-key="codex:reconnect:org"] .window-resets').innerText(), /Last known reset:/);
    const resetStates = await settings.evaluate(async () => {
      const { windowResetText } = await import('/i18n.js');
      const future = { resets_at: new Date(Date.now() + 86400000).toISOString() };
      const past = { resets_at: new Date(Date.now() - 3600000).toISOString() };
      return {
        missing: windowResetText({ resets_at: null }),
        invalid: windowResetText({ resets_at: 'not-a-time' }),
        stale: windowResetText(future, true),
        elapsed: windowResetText(past),
        fresh: windowResetText(future),
      };
    });
    assert.equal(resetStates.missing, 'Reset time unavailable');
    assert.equal(resetStates.invalid, 'Reset time unavailable');
    assert.match(resetStates.stale, /^Last known reset:/);
    assert.doesNotMatch(resetStates.stale, /Resets in/);
    assert.match(resetStates.elapsed, /^Reset time passed · awaiting refresh/);
    assert.match(resetStates.fresh, /^Resets in.* · /);
    const unavailableSnapshot = JSON.parse(JSON.stringify(data));
    unavailableSnapshot.accounts[3].windows[1].resets_at = 'invalid-reset-time';
    await settings.evaluate(s => window.testEmit('usage-updated', s), unavailableSnapshot);
    assert.equal(await idle.locator('.window-resets').count(), 0);
    assert.equal(await idle.locator('[data-disconnect]').count(), 1);
    assert.doesNotMatch(await idle.innerText(), /Reset time unavailable|Independent monitoring connected|Model-specific usage unavailable/);
    const expiredSnapshot = JSON.parse(JSON.stringify(data));
    expiredSnapshot.accounts[3].windows[1].resets_at = new Date(Date.now() - 60000).toISOString();
    await settings.evaluate(s => window.testEmit('usage-updated', s), expiredSnapshot);
    await idle.getByText(/Weekly: \d+% used · Reset time passed · awaiting refresh/).waitFor();
    assert.match(await idle.locator('.meter').innerText(), /0%/);
    const freshSnapshot = JSON.parse(JSON.stringify(data));
    freshSnapshot.accounts[3].provider = 'codex';
    freshSnapshot.accounts[3].windows = [freshSnapshot.accounts[3].windows[1]];
    await settings.evaluate(s => window.testEmit('usage-updated', s), freshSnapshot);
    const idleCodex = settings.locator('[data-key="codex:zero:org"]');
    await idleCodex.getByText(/Weekly: \d+% used · Resets in/).waitFor();
    assert.equal(await idleCodex.locator('.window-reset').count(), 1);
    await settings.evaluate(s => window.testEmit('usage-updated', s), data);
    await idle.locator('.window-resets').waitFor();
    const exhausted = JSON.parse(JSON.stringify(data));
    exhausted.accounts[3].windows[0].resets_at = new Date(Date.now() + 3600000).toISOString();
    exhausted.accounts[3].windows[1].used_percent = 100;
    await settings.evaluate(s => window.testEmit('usage-updated', s), exhausted);
    assert.equal(await idle.locator('.window-reset').count(), 1);
    assert.match(await idle.locator('.window-resets').innerText(), /^Weekly:/);
    assert.match(await idle.locator('.meter').innerText(), /100%/);
    const priorityCases = await settings.evaluate(async () => {
      const { visibleResetWindows } = await import('/i18n.js');
      const future = new Date(Date.now() + 86400000).toISOString();
      const session = {kind:'session', used_percent:50, resets_at:future};
      return [100,101,99.6,null].map(used => visibleResetWindows([session,
        ...(used === null ? [] : [{kind:'weekly',used_percent:used,resets_at:future}])]).map(w=>w.kind));
    });
    assert.deepEqual(priorityCases, [['weekly'],['weekly'],['session','weekly'],['session']]);
    const exhaustedCodex = JSON.parse(JSON.stringify(exhausted));
    exhaustedCodex.accounts[3].provider = 'codex';
    await settings.evaluate(s => window.testEmit('usage-updated', s), exhaustedCodex);
    assert.match(await idleCodex.locator('.window-resets').innerText(), /^Weekly:/);
    assert.equal(await idleCodex.locator('.window-reset').count(), 1);
    const cliOnly = JSON.parse(JSON.stringify(exhausted));
    cliOnly.accounts[3].monitor_connected = false;
    cliOnly.accounts[3].windows[0].used_percent = 100;
    await settings.evaluate(s => window.testEmit('usage-updated', s), cliOnly);
    assert.doesNotMatch(await idle.locator('.meter').innerText(), /Resets in/);
    assert.equal(await idle.locator('.window-reset').count(), 1);
    assert.match(await idle.locator('.window-resets').innerText(), /^Weekly: 100% used · Resets in/);
    const cliFallback = JSON.parse(JSON.stringify(cliOnly));
    cliFallback.accounts[3].monitor_connected = true;
    cliFallback.accounts[3].active = true;
    cliFallback.accounts[3].monitor_error = 'monitor_reconnect';
    await settings.evaluate(s => window.testEmit('usage-updated', s), cliFallback);
    assert.match(await idle.locator('.window-resets').innerText(), /^Weekly: 100% used · Resets in/);
    assert.match(await idle.innerText(), /Reconnect monitoring/);
    await settings.evaluate(s => window.testEmit('usage-updated', s), data);
    await idle.locator('.window-resets').waitFor();
    await settings.locator('[data-key="claude:inactive:org"] [data-monitor]').click();
    assert.match(await settings.locator('#add-hint').innerText(), /Finish the monitoring sign-in/);
    const called = await settings.evaluate(() => window.testCalls.filter(c => c.command === 'connect_monitoring' || c.command === 'switch_account'));
    assert.deepEqual(called, [{ command: 'connect_monitoring', args: { provider: 'claude', id: 'inactive:org' } }]);
    await settings.locator('[data-key="claude:connected:org"] [data-disconnect]').click();
    assert.match(await settings.locator('#add-hint').innerText(), /session revoked/);
    assert.deepEqual(await settings.evaluate(() => window.testCalls.filter(c => c.command === 'disconnect_monitoring')), [{ command: 'disconnect_monitoring', args: { provider: 'claude', id: 'connected:org' } }]);
    await settings.locator('[data-page="general"]').click();
    await settings.locator('#export-diagnostics').click();
    assert.match(await settings.locator('#diagnostics-status').innerText(), /Diagnostics saved: C:\\test\\SlopBar-Diagnostics.json/);
    await settings.evaluate(() => { window.testExportFail = true; });
    await settings.locator('#export-diagnostics').click();
    assert.match(await settings.locator('#diagnostics-status').innerText(), /Could not export diagnostics/);
    assert.doesNotMatch(await settings.locator('#diagnostics-status').innerText(), /PRIVATE/);
    assert.equal(await settings.locator('#export-diagnostics').isDisabled(), false);
    await settings.evaluate(() => {
      for (let i = 0; i < 25; i++) window.dispatchEvent(new ErrorEvent('error', { message: 'PRIVATE-message', filename: 'https://PRIVATE-url/token', error: new TypeError('PRIVATE-secret'), lineno: 42, colno: 3 }));
    });
    const frontend = await settings.evaluate(() => window.testCalls.filter(c => c.command === 'record_frontend_error'));
    assert.equal(frontend.length, 20);
    assert.deepEqual(frontend[0].args, { surface: 'settings', kind: 'type_error', line: 42, column: 3 });
    assert.doesNotMatch(JSON.stringify(frontend), /PRIVATE/);
    await settings.locator('[data-page="accounts"]').click();
    await settings.setViewportSize({ width: 760, height: 800 });
    assert.equal(await settings.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (process.env.UI_SCREENSHOT_DIR) await settings.screenshot({ path: path.join(process.env.UI_SCREENSHOT_DIR, 'slopbar-settings.png'), fullPage: true });

    const panel = await browser.newPage({ viewport: { width: 380, height: 1100 } });
    panel.on('pageerror', err => errors.push(String(err)));
    await setup(panel, 'panel');
    await panel.goto(base + '/index.html');
    await panel.locator('button.monitor').first().waitFor();
    assert.match(await panel.locator('[data-key="codex:reconnect:org"] .reset-grant').innerText(), /type unknown[\s\S]*usable now/);
    const inactive = panel.locator('[data-key="claude:inactive:org"]');
    assert.match(await inactive.innerText(), /Last confirmed 2h ago/);
    assert.equal(await inactive.locator('.win .reset').count(), 0);
    const panelIdle = panel.locator('[data-key="claude:zero:org"]');
    await panel.evaluate(s => window.testEmit('usage-updated', s), exhausted);
    assert.equal(await panelIdle.locator('.win').count(), 2); // Usage percentages stay visible.
    assert.equal(await panelIdle.locator('.win .reset').count(), 1);
    const eligible = JSON.parse(JSON.stringify(exhausted));
    eligible.accounts[3].windows[1].used_percent = 99.6;
    await panel.evaluate(s => window.testEmit('usage-updated', s), eligible);
    assert.equal(await panelIdle.locator('.win .reset').count(), 2);
    await panel.evaluate(s => window.testEmit('usage-updated', s), data);
    await runResetScheduleMatrix(settings, panel, data);
    await inactive.locator('button.monitor').click();
    assert.equal(await panel.locator('#panel-notice').isVisible(), true);
    await panel.evaluate(() => window.testEmit('monitoring-login', { error: 'monitor_wrong_account' }));
    assert.match(await panel.locator('#panel-notice').innerText(), /different account/);
    if (process.env.UI_SCREENSHOT_DIR) await panel.screenshot({ path: path.join(process.env.UI_SCREENSHOT_DIR, 'slopbar-panel.png'), fullPage: true });

    // Every flyout row carries its provider's mark, and each section its logo: one person's
    // Claude and Codex accounts share an email, so the name alone does not tell them apart.
    for (const section of await panel.locator('#accounts section').all()) {
      const provider = (await section.locator('.acc-head .logo').getAttribute('class')).split(' ')[1];
      for (const row of await section.locator('.acct').all()) {
        assert.equal(await row.locator(`.provider-mark.${provider}`).count(), 1, 'row marked with its provider');
      }
    }

    // Many accounts on a short screen: the header and the first provider stay on screen, the
    // account list scrolls, and the app is asked for the full unclipped height (it caps it).
    const many = JSON.parse(JSON.stringify(data));
    for (let i = 0; i < 12; i++) for (const provider of ['claude', 'codex']) {
      many.accounts.push({ ...JSON.parse(JSON.stringify(data.accounts[1])), id: `${provider}-${i}:org`, provider, email: `same${i}@example.invalid`, active: false });
    }
    await panel.setViewportSize({ width: 380, height: 500 });
    await panel.evaluate(() => { window.testCalls = []; });
    await panel.evaluate(s => window.testEmit('usage-updated', s), many);
    await panel.waitForTimeout(100);
    const box = await panel.locator('header').boundingBox();
    assert.ok(box && box.y >= 0 && box.y + box.height <= 500, 'flyout header stays visible');
    const firstHead = await panel.locator('#accounts section .acc-head').first().boundingBox();
    assert.ok(firstHead && firstHead.y + firstHead.height <= 500, 'first provider heading visible');
    const scroll = await panel.locator('#accounts').evaluate(el => ({ s: el.scrollHeight, c: el.clientHeight, o: getComputedStyle(el).overflowY }));
    assert.ok(scroll.s > scroll.c && scroll.o === 'auto', 'account list scrolls');
    const asked = await panel.evaluate(() => window.testCalls.filter(c => c.command === 'set_panel_height').map(c => c.args.height));
    assert.ok(asked.length && Math.max(...asked) > 500, `unclipped height requested (${asked})`);
    await panel.setViewportSize({ width: 380, height: 1100 });

    // An unused Codex window is reported as resetting one window length after the request: a
    // time that moves with every poll. It reads as not started, not as a countdown.
    const fetchedAt = new Date(Date.now() - 1000).toISOString();
    const unstartedCodex = JSON.parse(JSON.stringify(data));
    unstartedCodex.accounts = [{ ...unstartedCodex.accounts[3], provider: 'codex', id: 'idle:org', fetched_at: fetchedAt, stale: false, error: null, monitor_error: null,
      windows: [{ kind: 'weekly', used_percent: 0, window_seconds: 604800, resets_at: new Date(Date.parse(fetchedAt) + 604800000).toISOString() }] },
      { ...unstartedCodex.accounts[3], provider: 'codex', id: 'used:org', fetched_at: fetchedAt, stale: false, error: null, monitor_error: null,
      windows: [{ kind: 'weekly', used_percent: 75, window_seconds: 604800, resets_at: new Date(Date.parse(fetchedAt) + 5 * 86400000).toISOString() }] }];
    for (const page of [panel, settings]) await page.evaluate(s => window.testEmit('usage-updated', s), unstartedCodex);
    assert.match(await panel.locator('[data-key="codex:idle:org"] .win .reset').innerText(), /^Not started · begins on first use$/);
    assert.match(await panel.locator('[data-key="codex:used:org"] .win .reset').innerText(), /^Resets in 5d/);
    assert.match(await settings.locator('[data-key="codex:idle:org"] .window-reset').innerText(), /^Weekly: 0% used · Not started/);
    assert.match(await settings.locator('[data-key="codex:used:org"] .window-reset').innerText(), /^Weekly: 75% used · Resets in 5d/);
    for (const page of [panel, settings]) await page.evaluate(s => window.testEmit('usage-updated', s), data);

    // Every supported language supplies the monitoring controls and error strings.
    const translations = await panel.evaluate(async () => {
      const { t, setLanguage, LANGUAGES } = await import('/i18n.js');
      const keys = ['monitorDisconnect', 'monitorDisconnected', 'monitorRevokeFailed', 'monitor_uncertain', 'monitorConnect', 'monitorReconnect', 'monitorConnected', 'monitorPending', 'monitor_setup', 'monitor_reconnect', 'monitor_storage', 'monitor_network', 'monitor_wrong_account', 'monitor_shared_login', 'monitor_login_failed', 'monitor_busy', 'lastObserved', 'lastReading', 'usageUnknown', 'manualUpdates', 'resetTimeUnavailable', 'lastResetAt', 'resetAwaitingRefresh', 'windowNotStarted'];
      setLanguage('en'); const english = keys.map(k => t(k));
      return Object.keys(LANGUAGES).filter(l => l !== 'en').map(l => {
        setLanguage(l); return { l, values: keys.map(k => t(k)), english };
      });
    });
    for (const { l, values, english } of translations) for (let i = 0; i < values.length; i++) assert.notEqual(values[i], english[i], `${l} translation ${i}`);

    const dock = await browser.newPage();
    dock.on('pageerror', err => errors.push(String(err)));
    await setup(dock, 'dock');
    await dock.goto(base + '/index.html');
    await dock.locator('.unknown-mark').waitFor();
    assert.equal(await dock.locator('#compact .ring').count(), 1); // Fresh Claude; stale Codex is unknown.
    assert.deepEqual(errors, []);
    console.log('PASS: settings, flyout, and taskbar; monitoring action, stale readings, errors, manual updates, and minimum window width.');
  } finally {
    await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(err => { console.error(err); server.close(); process.exitCode = 1; });
