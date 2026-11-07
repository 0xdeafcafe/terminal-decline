import assert from 'node:assert/strict';
import { readFile, readdir, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const site = fileURLToPath(new URL('../', import.meta.url));
const dist = path.join(site, 'dist');
assert(!(await readdir(dist, { recursive: true })).some(name => /\.(?:m?js|js\.map)$/.test(name)), 'JavaScript must not be included in the Pages artifact.');
const index = await readFile(path.join(dist, 'index.html'), 'utf8');
const missing = await readFile(path.join(dist, '404.html'), 'utf8');
for (const html of [index, missing]) {
  assert(!/<script\b|<astro-island\b/i.test(html), 'The static site must ship no JavaScript.');
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
const data = JSON.parse(await readFile(path.join(site, 'src/data/comparison.json'), 'utf8'));
const readme = await readFile(path.join(site, '../README.md'), 'utf8');
assert(readme.includes(data.date), 'Comparison date no longer matches the README.');
for (const terminal of data.terminals) {
  const row = readme.split('\n').find(line => line.startsWith('|') && line.includes(terminal.name));
  assert(row, `Missing README row for ${terminal.name}`);
  const values = row.split('|').slice(2).map(cell => cell.replaceAll('*', '').trim());
  assert.deepEqual(values.slice(0, 6), [`${terminal.idle} MB`, `${terminal.scrollback} MB`, `${terminal.tabs} MB`, `${terminal.cat.toFixed(2)} s`, `${terminal.cpu.toFixed(2)} s`, `${terminal.idleCpu.toFixed(2)}%`], `Stale comparison for ${terminal.name}`);
  assert(capture.text.includes(`${terminal.idle} MB`) && capture.text.includes(`${terminal.cat.toFixed(2)} s`), `Stale capture for ${terminal.name}`);
}
console.log('Static pages, local links, CNAME, social card and captured benchmark values verified. No JavaScript shipped.');
