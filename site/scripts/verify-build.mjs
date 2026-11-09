import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, readdir, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const site = fileURLToPath(new URL('../', import.meta.url));
const dist = path.join(site, 'dist');
assert(!(await readdir(dist, { recursive: true })).some(name => /\.(?:m?js|js\.map)$/.test(name)), 'JavaScript must not be included in the Pages artifact.');
const index = await readFile(path.join(dist, 'index.html'), 'utf8');
const missing = await readFile(path.join(dist, '404.html'), 'utf8');
for (const html of [index, missing]) {
  assert(!/<astro-island\b/i.test(html), 'Framework islands must not ship.');
  const scripts = [...html.matchAll(/<script\b([^>]*)>/gi)];
  assert.equal(scripts.length, 1, 'Expected one clipboard handler.');
  assert(/data-install-copy\b/.test(scripts[0][1]) && !/\bsrc=/.test(scripts[0][1]), 'Unexpected browser script.');
  for (const match of html.matchAll(/(?:src|href)="(\/(?!\/)[^"?#]*)(?:[?#][^"]*)?"/g)) {
    const target = match[1].endsWith('/') ? `${match[1]}index.html` : match[1];
    assert((await stat(path.join(dist, target))).isFile(), `Missing asset or page: ${target}`);
  }
}
assert.equal((await readFile(path.join(dist, 'CNAME'), 'utf8')).trim(), 'decline.forbes.red');
assert(index.includes('og:image') && index.includes('/images/social.png'), 'Missing social image metadata.');
assert(missing.includes('noindex'), '404 should not be indexed.');
const capture = JSON.parse(await readFile(path.join(site, 'src/data/terminal-preview.json'), 'utf8'));
assert.equal(capture.rows.map(row => row.map(run => run.text).join('')).join('\n'), capture.text, 'Terminal runs differ from the captured text.');
// The README's 2 November table, which the hero's captured terminal shows.
const data = JSON.parse(await readFile(path.join(site, 'src/data/readme-2026-11-02.json'), 'utf8'));
const readme = await readFile(path.join(site, '../README.md'), 'utf8');
assert(readme.includes(data.date), 'Comparison date no longer matches the README.');
for (const terminal of data.terminals) {
  const row = readme.split('\n').find(line => line.startsWith('|') && line.includes(terminal.name));
  assert(row, `Missing README row for ${terminal.name}`);
  const values = row.split('|').slice(2).map(cell => cell.replaceAll('*', '').trim());
  assert.deepEqual(values.slice(0, 6), [`${terminal.idle} MB`, `${terminal.scrollback} MB`, `${terminal.tabs} MB`, `${terminal.cat.toFixed(2)} s`, `${terminal.cpu.toFixed(2)} s`, `${terminal.idleCpu.toFixed(2)}%`], `Stale comparison for ${terminal.name}`);
  assert(capture.text.includes(`${terminal.idle} MB`) && capture.text.includes(`${terminal.cat.toFixed(2)} s`), `Stale capture for ${terminal.name}`);
}

const completion = JSON.parse(await readFile(path.join(site, 'src/data/completion-preview.json'), 'utf8'));
assert.equal(createHash('sha256').update(await readFile(path.join(site, '..', completion.source))).digest('hex'), completion.sha256, 'Completion source changed; review the text transcription.');
assert(!index.includes('screenshot-autocomplete'), 'Terminal screenshots must not be shipped.');
assert(!(await readdir(path.join(dist, 'images'))).some(name => name.startsWith('screenshot-')), 'Obsolete terminal screenshot in build.');

// The chart: the latest comparison, tied to its own dated report.
const chart = JSON.parse(await readFile(path.join(site, 'src/data/comparison.json'), 'utf8'));
const reportName = chart.source.split('/').pop();
const report = await readFile(path.join(site, '../docs', reportName), 'utf8');
const reportMetrics = {
  idle: 'Idle footprint (MB)',
  tabs: 'Footprint with 4 tabs (MB)',
  scrollback: 'Footprint after `seq 1 10000` (MB)',
  cat: '`cat` 100 MB: wall time (s)',
  cpu: '`cat` 100 MB: terminal + WindowServer CPU (s)',
  idleCpu: 'Idle CPU (%)',
};
assert(report.includes(chart.date), 'Comparison date differs from its report.');
assert(report.includes(`Medians of **${chart.runs} rounds**`), 'Comparison rounds differ from its report.');
assert(!/M1 Max/.test(chart.machine) && /runner/.test(chart.machine), 'Label the chart with the machine it ran on.');
for (const [key, heading] of Object.entries(reportMetrics)) {
  const section = report.split(`### ${heading}\n`)[1]?.split('\n### ')[0];
  assert(section, `Missing metric: ${heading}`);
  for (const terminal of chart.terminals) {
    const row = section.split('\n').find(line => line.startsWith(`| ${terminal.name} |`));
    assert(row, `Missing result: ${terminal.name} / ${key}`);
    assert.equal(terminal[key], Number(row.split('|')[2].trim()), `Value differs from the report: ${terminal.name} / ${key}`);
  }
}
assert(index.includes('Warp ran with two windows.'), 'Missing Warp measurement limit.');
assert.equal(index.match(/data-comparison="/g)?.length, 1, 'One comparison chart.');
assert(index.includes(`data-comparison="${chart.date}"`), 'The chart must be the latest comparison.');
const [primary, method] = index.split('<details class="benchmark-method">');
assert(primary.includes(chart.machine.replace("'", '&#39;')) || primary.includes('GitHub-hosted macOS runner'), 'Name the runner above the chart.');
assert(method?.includes('comparison-2026-10-30.md') && method.includes('#benchmarks'), 'Earlier runs belong in the method notes as links.');
console.log('Static pages, links, assets, the README table and the dated comparison verified. No framework runtime shipped.');
