import { readFile, mkdir, unlink } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import sharp from 'sharp';

const site = fileURLToPath(new URL('../', import.meta.url));
const repo = path.dirname(site);
const output = path.join(site, 'public/images');
await mkdir(output, { recursive: true });
const assets = JSON.parse(await readFile(path.join(site, 'src/data/assets.json'), 'utf8'));
await Promise.all(Object.entries(assets).map(async ([name, source]) => {
  await sharp(path.join(repo, source)).resize({ width: 256, withoutEnlargement: true }).webp({ lossless: true }).toFile(path.join(output, name.replace(/\.png$/, '.webp')));
}));
// Remove obsolete generated screenshots from earlier builds.
for (const name of ['screenshot-autocomplete.png', 'screenshot-autocomplete.webp']) {
  await unlink(path.join(output, name)).catch(error => { if (error.code !== 'ENOENT') throw error; });
}
const social = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630">
<rect width="1200" height="630" fill="#092D2B"/>
<g transform="translate(70 60) scale(.13)"><path d="M294 400 L484 610 Q497 624 516 624 H738" fill="none" stroke="#B9E5CC" stroke-width="92" stroke-linecap="round" stroke-linejoin="round"/></g>
<g fill="#F2F7F4" font-family="-apple-system,Helvetica,Arial,sans-serif">
<text x="180" y="140" font-size="42">Terminal Decline</text>
<text x="80" y="285" font-size="84" font-weight="800" fill="#B9E5CC">A peppy terminal</text>
<text x="80" y="375" font-size="84" font-weight="800" fill="#B9E5CC">for macOS.</text>
<text x="84" y="455" font-size="32">26 MB idle. 0.58 s to cat 100 MB.</text>
<text x="84" y="548" font-size="24" fill="#B9E5CC">Measured 2 November 2026 on an Apple M1 Max.</text>
</g></svg>`;
await sharp(Buffer.from(social)).png().toFile(path.join(output, 'social.png'));
console.log(`Prepared ${Object.keys(assets).length} pin assets and social card.`);
