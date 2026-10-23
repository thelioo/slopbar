// Synthetic presentation checks only. No authentication commands or provider requests.
const assert = require('node:assert/strict');

exports.runResetScheduleMatrix = async function(settings, panel, original) {
  const now = Date.now();
  const future = new Date(now + 86400000).toISOString();
  const past = new Date(now - 3600000).toISOString();
  const window = (kind, used_percent, resets_at = future) => ({kind, used_percent, resets_at,
    window_seconds: kind === 'session' ? 18000 : 604800});
  const cases = [
    ['tied100', [window('session',100),window('weekly',100)], ['Weekly']],
    ['above100', [window('session',100),window('weekly',101)], ['Weekly']],
    ['rounded100', [window('session',100),window('weekly',99.6)], ['Session (5h)','Weekly']],
    ['sessionBinds', [window('session',80),window('weekly',20)], ['Session (5h)','Weekly']],
    ['zeroNullSession', [window('session',0,null),window('weekly',0)], ['Weekly']],
    ['noWeekly', [window('session',42)], ['Session (5h)']],
    ['invalidSession', [window('session',42,'invalid-time'),window('weekly',60)], ['Weekly']],
    ['noValidTimes', [window('session',42,null),window('weekly',60,'invalid-time')], []],
    ['elapsedSession', [window('session',42,past),window('weekly',60)], ['Session (5h)','Weekly']],
    ['elapsedWeekly', [window('session',100),window('weekly',100,past)], ['Weekly']],
    ['stale', [window('session',42),window('weekly',60)], ['Session (5h)','Weekly'], true],
    ['recovery', [window('session',42),window('weekly',40)], ['Session (5h)','Weekly']],
  ];
  const modes = [
    ['cliOnly',false,null,true],
    ['monitoring',true,null,false],
    ['activeCliFallback',true,'monitor_reconnect',true],
  ];
  const failures=[];
  let passed=0;
  for (const provider of ['claude','codex']) for (const [mode, connected, monitor_error, active] of modes) {
    for (const [name, windows, labels, stale=false] of cases) {
      const title=`${provider}/${mode}/${name}`;
      const account={id:'schedule-fixture',provider,email:'schedule@example.invalid',alias:null,org:null,
        plan:'test',active,monitor_connected:connected,monitor_error,error:null,stale,
        sources:active?['Windows']:[],fetched_at:new Date(now-(stale?7200000:1000)).toISOString(),
        credits:null,resets:[],windows};
      const snapshot={updated_at:new Date(now).toISOString(),accounts:[account]};
      await settings.evaluate(s=>window.testEmit('usage-updated',s),snapshot);
      await panel.evaluate(s=>window.testEmit('usage-updated',s),snapshot);
      try {
        const row=settings.locator(`[data-key="${provider}:schedule-fixture"]`);
        const flyout=panel.locator(`[data-key="${provider}:schedule-fixture"]`);
        const lines=await row.locator('.window-reset').allTextContents();
        assert.equal(lines.length,labels.length,'known Settings schedules remain visible');
        for(let i=0;i<labels.length;i++) {
          assert.ok(lines[i].startsWith(labels[i]+':'),`Settings retains ${labels[i]}`);
          assert.ok(/Resets in|Last known reset|Reset time passed/.test(lines[i]),'schedule has meaningful timing');
        }
        assert.equal(await row.locator('.window-resets').count(),labels.length?1:0,'no empty schedule group');
        assert.doesNotMatch(await row.locator('.meter').innerText(),/Resets in|Resets now|Reset time passed/,'no duplicate headline countdown');
        assert.equal(await row.locator('[data-disconnect]').count(),connected?1:0);
        assert.equal(await row.locator('[data-monitor]').count(),(!connected||monitor_error)?1:0);
        if(monitor_error) assert.match(await row.innerText(),/Reconnect monitoring|sign-in needs reconnecting/);
        if(stale) assert.match(await row.innerText(),/Last confirmed 2h ago/);
        // The headline is the binding (fullest) window; a tie goes to the overall week.
        const most=Math.max(...windows.map(w=>w.used_percent));
        const top=windows.find(w=>w.kind==='weekly'&&w.used_percent===most)??windows.find(w=>w.used_percent===most);
        assert.match(await row.locator('.meter').innerText(),new RegExp(Math.round(most)+'%'));
        assert.match(await row.locator('.window-label').innerText(),top.kind==='weekly'?/Weekly · used/:/Session \(5h\) · used/);
        for(let i=0;i<labels.length;i++) {
          const w=windows.find(w=>w.kind===(labels[i]==='Weekly'?'weekly':'session'));
          assert.ok(lines[i].startsWith(`${labels[i]}: ${Math.round(w.used_percent)}% used · `),'each schedule states its own usage');
        }
        assert.equal(await flyout.locator('.win').count(),windows.length,'all reported usage windows remain');
        assert.deepEqual(await flyout.locator('.win .row b').allTextContents(),
          windows.map(w=>Math.round(w.used_percent)+'%'),'deadlines never change reported percentages');
        const panelLines=await flyout.locator('.win .reset').allTextContents();
        assert.equal(panelLines.length,stale?0:labels.length,'flyout keeps every eligible reset');
        if(name.startsWith('elapsed')) {
          assert.ok(lines.some(l=>l.includes('Reset time passed · awaiting refresh')),'Settings marks elapsed deadline');
          assert.ok(panelLines.some(l=>l.includes('Reset time passed · awaiting refresh')),'flyout marks elapsed deadline');
          assert.doesNotMatch(await flyout.innerText(),/Resets now/);
        }
        for(const reset of await flyout.locator('.win .reset').all()) {
          assert.ok(Number.isFinite(Date.parse(await reset.getAttribute('title'))),'flyout UTC reset tooltip retained');
        }
        if(stale) for(const line of lines) assert.match(line,/Last known reset:/);
        for(const reset of await row.locator('.window-reset').all()) {
          assert.ok(Number.isFinite(Date.parse(await reset.getAttribute('title'))),'UTC reset tooltip retained');
        }
        passed++;
      } catch(error) { failures.push(`${title}: ${error.message}`); }
    }
  }
  await settings.evaluate(s=>window.testEmit('usage-updated',s),original);
  await panel.evaluate(s=>window.testEmit('usage-updated',s),original);
  const formatter = await panel.evaluate(async () => {
    const {resetsIn}=await import('/i18n.js');
    return {elapsed:resetsIn(new Date(Date.now()-3600000).toISOString()),
      near:resetsIn(new Date(Date.now()+15000).toISOString()), invalid:resetsIn('invalid')};
  });
  assert.match(formatter.elapsed,/Reset time passed · awaiting refresh/);
  assert.match(formatter.near,/Resets in/);
  assert.doesNotMatch(formatter.near,/Resets now/);
  assert.equal(formatter.invalid,'Reset time unavailable');
  if(failures.length) {
    console.error(JSON.stringify({reset_schedule_matrix:{passed,failed:failures.length},examples:failures.slice(0,16)},null,2));
    throw new Error('Reset schedule matrix failed');
  }
  console.log(`PASS: ${passed} reset schedule cases across Claude/Codex, CLI-only, monitoring and active CLI fallback.`);
};
