// Checks src/i18n.js: every language has every English key, with the same {placeholders}.
// Run `node scripts/check-i18n.cjs`; `--missing` prints the missing keys as JSON instead.
const fs = require('node:fs');
const path = require('node:path');

const source = fs.readFileSync(path.resolve(__dirname, '../src/i18n.js'), 'utf8');
const start = source.indexOf('const MESSAGES = ');
const end = source.slice(start).search(/\n\};\r?\n/) + start;
if (start < 0 || end < start) throw new Error('MESSAGES not found in src/i18n.js');
const MESSAGES = new Function(`return ${source.slice(start + 'const MESSAGES = '.length, end + 3)}`)();

const placeholders = (s) => [...new Set(String(s).match(/\{\w+\}/g) ?? [])].sort().join(' ');
const en = MESSAGES.en;
const missing = {};
const problems = [];
for (const [lang, msgs] of Object.entries(MESSAGES)) {
  if (lang === 'en') continue;
  for (const key of Object.keys(en)) {
    if (!(key in msgs)) { (missing[lang] ??= {})[key] = en[key]; continue; }
    if (placeholders(msgs[key]) !== placeholders(en[key])) {
      problems.push(`${lang}.${key}: placeholders "${placeholders(msgs[key])}" differ from en "${placeholders(en[key])}"`);
    }
  }
  for (const key of Object.keys(msgs)) if (!(key in en)) problems.push(`${lang}.${key}: not an English key`);
}

if (process.argv.includes('--missing')) {
  process.stdout.write(JSON.stringify(missing, null, 1));
  process.exit(0);
}
for (const [lang, keys] of Object.entries(missing)) problems.push(`${lang}: ${Object.keys(keys).length} English keys missing (${Object.keys(keys).slice(0, 5).join(', ')}…)`);
if (problems.length) {
  console.error(problems.join('\n'));
  process.exit(1);
}
console.log(`i18n ok: ${Object.keys(MESSAGES).length} languages, ${Object.keys(en).length} keys each`);
