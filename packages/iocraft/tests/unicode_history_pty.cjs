// Real PTY/ConPTY reduction. Traced and untraced runs are distinct diagnostic
// cases; every case retains the same strict input-row ownership assertion.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const fromHost = name => require(process.env.IOCRAFT_TERMINAL_MODULES
  ? path.join(process.env.IOCRAFT_TERMINAL_MODULES, name) : name);
const pty = fromHost('node-pty');
const { Terminal } = fromHost('@xterm/headless');
const { Unicode11Addon } = fromHost('@xterm/addon-unicode11');
const [fixture, backend = 'bundled'] = process.argv.slice(2);
assert(fixture && ['bundled', 'system'].includes(backend));
const terminal = new Terminal({ cols: 120, rows: 40, scrollback: 10000, allowProposedApi: true,
  windowsPty: process.platform === 'win32'
    ? { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } : undefined });
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = '11';
const environment = { ...process.env, TERM: 'xterm-256color' };
delete environment.ELECTRON_RUN_AS_NODE;
const child = pty.spawn(path.resolve(fixture), [], { cols: 120, rows: 40,
  cwd: path.dirname(path.resolve(fixture)), env: environment,
  useConpty: true, useConptyDll: backend === 'bundled' });
const wire = [];
let pending = 0, exited = false, exitCode;
child.onData(data => {
  wire.push({ data }); pending++;
  terminal.write(data, () => pending--);
});
terminal.onData(data => { wire.push({ reply: data }); child.write(data); });
if (process.platform === 'win32') terminal.parser.registerCsiHandler({ final: 'c' }, params => {
  if (!params.length || (params.length === 1 && params[0] === 0)) {
    child.write('\x1b[?61;4c'); return true;
  }
  return false;
});
child.onExit(event => { exited = true; exitCode = event.exitCode; });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function rows() {
  const buffer = terminal.buffer.active;
  return Array.from({ length: buffer.length }, (_, i) => buffer.getLine(i).translateToString(true));
}
async function expect(condition, description) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    await sleep(20);
    if (!pending && condition(rows())) return;
    assert(!exited, `fixture exited while waiting for ${description}`);
  }
  throw new Error(`${description}\n${rows().join('\n')}`);
}
async function input(text) {
  await expect(lines => lines.some(line => line.startsWith(`❯ ${text}`)),
    `input must remain on its prompt row: ${text}`);
}
(async () => {
  try {
    await expect(lines => lines.includes('UnicodeFooter p0 120x40'), 'initial frame');
    child.write('DraftBefore');
    await input('DraftBefore');
    child.write('\x1bOP');
    await expect(lines => lines.includes('UnicodeFooter p1 120x40'), 'live preview frame');
    await input('DraftBefore');
    child.write('\x1bOQ');
    await expect(lines => lines.includes('UnicodeFooter p2 120x40'), 'history handoff frame');
    await input('DraftBefore');
    child.write('-After');
    await input('DraftBefore-After');
    child.write('\x1b');
    for (let i = 0; i < 250; i++) {
      if (exitCode === undefined && process.platform === 'win32') exitCode = child._agent?.exitCode;
      if (exited && exitCode !== undefined && !pending) break;
      await sleep(20);
    }
    assert(exited && exitCode === 0, `normal exit required: ${exitCode}`);
    console.log(JSON.stringify({ backend, case: process.env.IOCRAFT_UNICODE_HISTORY_CASE,
      trace: Boolean(process.env.IOCRAFT_FRAME_WRITE_TRACE), inputOwnership: true, exitCode }));
  } finally {
    if (!exited) {
      child.write('\x1b');
      for (let i = 0; i < 100 && !exited; i++) await sleep(20);
    }
    if (!exited) child.kill();
    terminal.dispose();
    if (process.env.IOCRAFT_WIRE_TRACE)
      fs.writeFileSync(process.env.IOCRAFT_WIRE_TRACE, wire.map(row => JSON.stringify(row)).join('\n') + '\n');
  }
})().then(() => process.exit(0)).catch(error => { console.error(error); process.exit(1); });
