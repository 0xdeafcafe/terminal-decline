import assert from 'node:assert/strict';
import { readFile, readdir, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const files = await readdir(dist, { recursive: true });
const pages = await Promise.all(files.filter(name => name.endsWith('.html')).map(name => readFile(path.join(dist, name), 'utf8')));
assert(pages.every(html => !/<astro-island\b/i.test(html)), 'Hydrated framework code must not ship.');
for (const html of pages) {
  for (const [, attrs] of html.matchAll(/<script\b([^>]*)>/gi)) {
    assert(/data-install-copy\b/.test(attrs) && !/\bsrc=/.test(attrs), 'Only the inline clipboard handler is expected.');
  }
}
// Astro 7 emits an unused React renderer chunk even without hydrated islands.
// Remove it from the Pages artifact only after proving no page references it.
const scripts = files.filter(name => /\.(?:m?js|js\.map)$/.test(name));
for (const name of scripts) {
  assert(pages.every(html => !html.includes(path.basename(name))), `A page references ${name}; refusing to remove it.`);
  await rm(path.join(dist, name));
}
console.log(`Removed ${scripts.length} unreferenced runtime asset(s). Only the inline clipboard handler ships.`);
