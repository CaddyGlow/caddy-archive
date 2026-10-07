// Minimal local-only static server for the WASM regression harness.
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';

const root = resolve(process.argv[2] ?? 'crates/archive-wasm/tests');
const port = Number(process.argv[3] ?? 8786);
const packageRoot = resolve(process.argv[4] ?? '/tmp/archive-browser/pkg');
const helperRoot = resolve(root, '../js');
const archiveFixtures = resolve('crates/archive-core/tests/fixtures/sevenz-upstream/resources');
http.createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const route = [['/pkg/', packageRoot], ['/js/', helperRoot], ['/archive-fixtures/', archiveFixtures]]
      .find(([prefix]) => pathname.startsWith(prefix));
    const base = route ? route[1] : root;
    const relative = route ? pathname.slice(route[0].length - 1) : pathname;
    const path = resolve(base, '.' + (relative === '/' ? '/browser.html' : relative));
    if (!path.startsWith(base + sep)) {
      response.writeHead(403).end();
      return;
    }
    const types = {'.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm'};
    response.setHeader('Content-Type', types[extname(path)] ?? 'application/octet-stream');
    response.end(await readFile(path));
  } catch (_) {
    response.writeHead(404).end();
  }
}).listen(port, '127.0.0.1', () => console.log(`http://127.0.0.1:${port}`));
