// Real-PTY history transaction regression. On Windows this may run with
// ELECTRON_RUN_AS_NODE=1 using VS Code's matching node-pty/xterm modules.
// IOCRAFT_TERMINAL_MODULES points at node_modules (or node_modules.asar).
// Usage: node inline_history_pty.cjs <fixture.exe> [bundled|system] [resize|rapid|tiny]
// Resize modes are acceptance diagnostics; a failing run must not be reported
// as a passing regression. IOCRAFT_WIRE_TRACE optionally saves fixture VT bytes.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const assert = require('node:assert/strict');
const modules = process.env.IOCRAFT_TERMINAL_MODULES;
const fromHost = name => require(modules ? path.join(modules, name) : name);
const pty = fromHost('node-pty');
const { Terminal } = fromHost('@xterm/headless');
const [fixture, mode = 'bundled', resize] = process.argv.slice(2);
assert(fixture, 'pass an explicitly built fixture executable');
assert(['bundled', 'system'].includes(mode), `unknown backend: ${mode}`);
assert(!resize || ['resize', 'rapid', 'tiny'].includes(resize), `unknown scenario: ${resize}`);
const trace = record => {
  if (process.env.IOCRAFT_WIRE_TRACE)
    fs.appendFileSync(process.env.IOCRAFT_WIRE_TRACE, JSON.stringify({ time: Date.now(), ...record }) + '\n');
};
const terminal = new Terminal({ cols: 240, rows: 40, scrollback: 10000,
  allowProposedApi: true, reflowCursorLine: mode !== 'system',
  windowsPty: process.platform === 'win32' ? { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } : undefined });
const { Unicode11Addon } = fromHost('@xterm/addon-unicode11');
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = '11';
console.log(JSON.stringify({ host: { platform: process.platform, release: os.release(),
  node: process.versions.node, electron: process.versions.electron,
  pty: fromHost('node-pty/package.json').version,
  xterm: fromHost('@xterm/headless/package.json').version }, mode, scenario: resize || 'transactions' }));
const env = { ...process.env, TERM: 'xterm-256color' };
delete env.ELECTRON_RUN_AS_NODE;
delete env.NO_COLOR;
const child = pty.spawn(path.resolve(fixture), [], { cols: 240, rows: 40,
  cwd: path.dirname(path.resolve(fixture)), env, useConpty: true, useConptyDll: mode === 'bundled' });
let pending = 0, lastData = Date.now(), exited = false;
let exitCode;
let lastResize;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function snapshot() {
  const buffer = terminal.buffer.active;
  return Array.from({ length: buffer.length }, (_, i) => buffer.getLine(i).translateToString(true)).join('\n');
}
// Observe the host buffer on both sides of frontend resize, before requesting
// the child resize. This distinguishes host reflow from application repaint;
// these coordinates are diagnostic evidence, not a runtime geometry oracle.
function geometry(stage) {
  const buffer = terminal.buffer.active;
  const statusRows = [];
  for (let row = 0; row < buffer.length; row++) {
    if (buffer.getLine(row).translateToString(true).includes('StatusMarker')) statusRows.push(row);
  }
  const result = { stage, cols: terminal.cols, rows: terminal.rows, baseY: buffer.baseY,
    cursor: [buffer.cursorX, buffer.baseY + buffer.cursorY], statusRows,
    statusRowsInScrollback: statusRows.filter(row => row < buffer.baseY) };
  trace({ geometry: result });
  return result;
}
child.onData(data => {
  trace({ data });
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
child.onExit(event => { exited = true; exitCode = event.exitCode; });
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
  for (const marker of ['ShellHistoryBeforeScode', 'StatusMarker', 'TodoMarker', 'FooterMarker', 'DraftSurvives']) {
    assert.equal(text.split(marker).length - 1, 1,
      `${label}: ${marker}\nresize geometry: ${JSON.stringify(lastResize)}\n${text}`);
  }
  assert(text.includes(`StatusMarker phase=${phase}`), `${label}: stale status phase\n${text}`);
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
    await settle('FooterMarker p0 a0 240x40');
    child.write('DraftSurvives');
    await settle('DraftSurvives');
    check('initial', 0, []);
    child.write('\x1bOQ'); // F2, first hook leaves an unfinished line.
    await settle('FooterMarker p2 a0 240x40');
    check('partial', 2, ['SharedPartial']);
    child.write('\x1bOR'); // F3, independent hook must continue that line.
    await settle('FooterMarker p3 a0 240x40');
    check('cross-hook continuation', 3, ['SharedPartialJoined']);
    child.write('\x1bOS'); // F4, two hook batches in one frame.
    await settle('FooterMarker p4 a0 240x40');
    check('same-frame batches', 4, ['SharedPartialJoined', 'SameBatch:end']);
    child.write('\x1b[15~'); // F5, buffered stdout followed by stderr.
    await settle('FooterMarker p5 a0 240x40');
    const committed = ['SharedPartialJoined', 'SameBatch:end', 'StdoutPrefix:StderrSuffix'];
    check('cross-stream order', 5, committed);
    if (resize) {
      let acknowledgment = 0;
      function resizeTo(cols, rows) {
        const before = geometry('before frontend resize');
        trace({ resize: [cols, rows] });
        terminal.resize(cols, rows);
        lastResize = { before, afterFrontend: geometry('after frontend resize, before child resize') };
        child.resize(cols, rows);
      }
      async function acknowledge(cols, rows) {
        const started = Date.now();
        child.write('\x1b[17~'); // F6: must be processed after the resize request.
        await settle(`FooterMarker p5 a${++acknowledgment} ${cols}x${rows}`, cols, started);
        lastResize.afterRedraw = geometry('after acknowledged redraw');
      }
      const matrix = [[100,40], [240,40], [60,40], [240,40], [60,18], [240,40]];
      if (resize === 'rapid') {
        for (const [cols, rows] of matrix) {
          resizeTo(cols, rows);
          await sleep(5);
        }
        await acknowledge(240, 40);
        check('rapid resize', 5, committed);
      }
      // A subsequent slow round trip also detects latent corruption left by
      // a rapid round trip whose final dimensions matched its initial ones.
      for (const [cols, rows] of resize === 'tiny' ? [[30,8], [240,40]] : matrix) {
        resizeTo(cols, rows);
        await acknowledge(cols, rows);
        check(`resize ${cols}x${rows}`, 5, committed);
      }
    }
    child.write('\x1b');
    for (let i = 0; i < 100 && !exited; i++) await sleep(20);
    assert(exited, 'fixture failed to exit');
    assert.equal(exitCode, 0, 'fixture exited unsuccessfully');
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
