import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const profile = await mkdtemp(join(tmpdir(), 'archive-chromium-'));
const port = Number(process.env.ARCHIVE_CDP_PORT ?? 9786);
const browser = spawn(process.env.ARCHIVE_BROWSER ?? 'chromium', ['--headless=new', '--no-sandbox', '--disable-gpu',
  `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, 'about:blank'], {stdio: 'ignore'});
let socket;
try {
  let targets;
  for (let i = 0; i < 100; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); break; }
    catch (_) { await new Promise(resolve => setTimeout(resolve, 100)); }
  }
  const target = targets?.find(item => item.type === 'page');
  if (!target) throw new Error('Chromium debugging endpoint unavailable');
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {socket.onopen = resolve; socket.onerror = reject;});
  let id = 0;
  const pending = new Map();
  socket.onmessage = ({data}) => {
    const message = JSON.parse(data);
    if (message.id) {
      const request = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) request?.reject(new Error(JSON.stringify(message.error)));
      else request?.resolve(message.result);
    }
  };
  const command = (method, params = {}) => new Promise((resolve, reject) => {
    const requestId = ++id;
    pending.set(requestId, {resolve, reject});
    socket.send(JSON.stringify({id: requestId, method, params}));
  });
  await command('Page.navigate', {url: process.argv[2] ?? 'http://127.0.0.1:8786'});
  let result;
  for (let i = 0; i < 200; i++) {
    await new Promise(resolve => setTimeout(resolve, 100));
    const value = await command('Runtime.evaluate', {
      expression: 'document.querySelector("#result")?.textContent', returnByValue: true,
    });
    const text = value.result?.value;
    if (text && text !== 'RUNNING') { result = JSON.parse(text); break; }
  }
  console.log(JSON.stringify(result ?? {ok: false, error: 'observation timeout'}));
  if (!result?.ok) process.exitCode = 1;
} finally {
  socket?.close();
  browser.kill('SIGTERM');
  await new Promise(resolve => browser.once('exit', resolve));
  await rm(profile, {recursive: true, force: true, maxRetries: 5, retryDelay: 100});
}
