// Saves the share images: serves the built site, screenshots /og-card/wide/ and /og-card/square/ at their exact sizes
// with headless Chrome, writes og.png and og-square.png into the build, and removes the card pages from it.
// Usage: node scripts/og-capture.mjs <built-site-dir>   (CHROME=path/to/chrome to override the browser)
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, rm, stat } from 'node:fs/promises';
import { extname, join, resolve } from 'node:path';
import cards from '../src/data/og-cards.json' with { type: 'json' };

const dir = resolve(process.argv[2] ?? 'dist');
const base = '/r3cui/';
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.webp': 'image/webp', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png' };
const server = createServer(async (req, res) => {
  let path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (!path.startsWith(base)) { res.writeHead(404).end(); return; }
  path = join(dir, path.slice(base.length));
  try { if ((await stat(path)).isDirectory()) path = join(path, 'index.html'); res.writeHead(200, { 'Content-Type': types[extname(path)] ?? 'application/octet-stream' }).end(await readFile(path)); }
  catch { res.writeHead(404).end(); }
}).listen(0, '127.0.0.1');
await new Promise(r => server.once('listening', r));
const port = server.address().port;

const chrome = process.env.CHROME ?? ['google-chrome', 'chromium', 'chromium-browser'].find(c => spawnSync('which', [c]).status === 0);
if (!chrome) throw new Error('no Chrome found; set CHROME');
const shoot = (format, out) => new Promise((ok, fail) => {
  const { width, height } = cards[format];
  const p = spawn(chrome, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1', `--window-size=${width},${height}`,
    '--virtual-time-budget=4000', `--screenshot=${out}`, `http://127.0.0.1:${port}${base}og-card/${format}/`], { stdio: 'ignore' });
  p.on('exit', code => (code === 0 ? ok() : fail(new Error(`chrome exited ${code} for ${format}`))));
});
await shoot('wide', join(dir, 'og.png'));
await shoot('square', join(dir, 'og-square.png'));
server.close();
await rm(join(dir, 'og-card'), { recursive: true, force: true });
console.log(`wrote ${join(dir, 'og.png')} and ${join(dir, 'og-square.png')}`);
