// Real-PTY history transaction regression. On Windows this may run with
// ELECTRON_RUN_AS_NODE=1 using VS Code's matching node-pty/xterm modules.
// IOCRAFT_TERMINAL_MODULES points at node_modules (or node_modules.asar).
// Usage: node inline_history_pty.cjs <fixture.exe> [bundled|system] [resize]
const path = require('node:path');
const assert = require('node:assert/strict');
const modules = process.env.IOCRAFT_TERMINAL_MODULES;
const fromHost = name => require(modules ? path.join(modules, name) : name);
const pty = fromHost('node-pty');
const { Terminal } = fromHost('@xterm/headless');
const [fixture, mode = 'bundled', resize] = process.argv.slice(2);
assert(fixture, 'pass an explicitly built fixture executable');
const terminal = new Terminal({ cols: 240, rows: 40, scrollback: 10000,
  allowProposedApi: true, reflowCursorLine: mode !== 'system',
  windowsPty: process.platform === 'win32' ? { backend: 'conpty', buildNumber: 26200 } : undefined });
const { Unicode11Addon } = fromHost('@xterm/addon-unicode11');
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = '11';
const env = { ...process.env, TERM: 'xterm-256color' };
delete env.ELECTRON_RUN_AS_NODE;
delete env.NO_COLOR;
const child = pty.spawn(path.resolve(fixture), [], { cols: 240, rows: 40,
  cwd: path.dirname(path.resolve(fixture)), env, useConpty: true, useConptyDll: mode === 'bundled' });
let pending = 0, lastData = Date.now(), exited = false;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function snapshot() {
  const buffer = terminal.buffer.active;
  return Array.from({ length: buffer.length }, (_, i) => buffer.getLine(i).translateToString(true)).join('\n');
}
child.onData(data => {
  lastData = Date.now(); pending++;
  terminal.write(data, () => pending--);
});
terminal.onData(data => child.write(data));
// Match VS Code's ConPTY device-attributes reply, not xterm's default DA.
if (process.platform === 'win32') terminal.parser.registerCsiHandler({ final: 'c' }, params => {
  if (!params.length || (params.length === 1 && params[0] === 0)) {
    child.write('\x1b[?61;4c'); return true;
  }
  return false;
});
child.onExit(() => { exited = true; });
async function settle(marker, width, notBefore = 0) {
  const start = Date.now();
  while (Date.now() - start < 15000) {
    await sleep(25);
    const text = snapshot();
    if (!pending && lastData >= notBefore && Date.now() - lastData >= 250 && text.includes(marker)
      && (!width || text.split('\n').includes('─'.repeat(width)))) return;
    assert(!exited, `fixture exited while waiting for ${marker}`);
  }
  throw new Error(`timed out waiting for ${marker}\n${snapshot()}`);
}
function check(label, phase, committed) {
  const text = snapshot();
  for (const marker of ['ShellHistoryBeforeScode', `StatusMarker phase=${phase}`, 'TodoMarker', 'FooterMarker', 'DraftSurvives']) {
    assert.equal(text.split(marker).length - 1, 1, `${label}: ${marker}\n${text}`);
  }
  const lines = text.split('\n');
  for (let index = 0; index < 70; index++) {
    assert.equal(lines.filter(line => line === `Earlier history line ${index}`).length, 1,
      `${label}: history ${index} was lost or duplicated\n${text}`);
  }
  for (const line of committed) assert.equal(lines.filter(item => item === line).length, 1,
    `${label}: expected committed line ${line}\n${text}`);
  console.log(JSON.stringify({ mode, label, historyLines: 70, liveFrameCopies: 1, committed }));
}
(async () => {
  try {
    await settle('StatusMarker phase=0');
    child.write('DraftSurvives');
    await settle('DraftSurvives');
    check('initial', 0, []);
    child.write('\x1bOQ'); // F2, first hook leaves an unfinished line.
    await settle('StatusMarker phase=2');
    check('partial', 2, ['SharedPartial']);
    child.write('\x1bOR'); // F3, independent hook must continue that line.
    await settle('StatusMarker phase=3');
    check('cross-hook continuation', 3, ['SharedPartialJoined']);
    child.write('\x1bOS'); // F4, two hook batches in one frame.
    await settle('StatusMarker phase=4');
    check('same-frame batches', 4, ['SharedPartialJoined', 'SameBatch:end']);
    child.write('\x1b[15~'); // F5, buffered stdout followed by stderr.
    await settle('StatusMarker phase=5');
    const committed = ['SharedPartialJoined', 'SameBatch:end', 'StdoutPrefix:StderrSuffix'];
    check('cross-stream order', 5, committed);
    if (resize) {
      for (const [cols, rows] of [[100,40], [240,40], [60,40], [240,40], [60,18], [240,40]]) {
        const started = Date.now();
        terminal.resize(cols, rows); child.resize(cols, rows);
        await settle('StatusMarker phase=5', cols, started);
        check(`resize ${cols}x${rows}`, 5, committed);
      }
    }
    child.write('\x1b');
    for (let i = 0; i < 100 && !exited; i++) await sleep(20);
    assert(exited, 'fixture failed to exit');
  } finally {
    // Give the owned fixture a normal exit even when an assertion fails;
    // abruptly closing a live ConPTY can hang native-host teardown.
    if (!exited) {
      child.write('\x1b');
      for (let i = 0; i < 100 && !exited; i++) await sleep(20);
    }
    if (!exited) child.kill();
    terminal.dispose();
  }
})().then(() => process.exit(0)).catch(error => { console.error(error); process.exit(1); });
