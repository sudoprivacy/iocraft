// Diagnostic control: the fixture never repaints after its first frame.
// A resize failure here is still a failure, not an accepted renderer behavior.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const assert = require('node:assert/strict');
const modules = process.env.IOCRAFT_TERMINAL_MODULES;
const fromHost = name => require(path.join(modules, name));
const pty = fromHost('node-pty');
const { Terminal } = fromHost('@xterm/headless');
const [fixture, mode = 'bundled', cursorMode = 'top'] = process.argv.slice(2);
assert(fixture && ['bundled', 'system'].includes(mode));
const ack = path.join(os.tmpdir(), `iocraft-static-${process.pid}-${mode}.txt`);
assert(!fs.existsSync(ack), 'diagnostic requires a fresh acknowledgment path');
const terminal = new Terminal({ cols: 240, rows: 40, scrollback: 10000,
  allowProposedApi: true, reflowCursorLine: mode !== 'system',
  windowsPty: { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } });
const { Unicode11Addon } = fromHost('@xterm/addon-unicode11');
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = '11';
const env = { ...process.env, TERM: 'xterm-256color', IOCRAFT_STATIC_ACK: ack, IOCRAFT_STATIC_CURSOR: cursorMode };
delete env.ELECTRON_RUN_AS_NODE;
delete env.NO_COLOR;
const child = pty.spawn(path.resolve(fixture), [], { cols: 240, rows: 40,
  cwd: path.dirname(path.resolve(fixture)), env, useConpty: true, useConptyDll: mode === 'bundled' });
let pending = 0, lastData = Date.now(), exited = false, exitCode;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function trace(record) {
  if (process.env.IOCRAFT_WIRE_TRACE)
    fs.appendFileSync(process.env.IOCRAFT_WIRE_TRACE, JSON.stringify({ time: Date.now(), ...record }) + '\n');
}
child.onData(data => {
  trace({ data }); lastData = Date.now(); pending++;
  terminal.write(data, () => pending--);
});
child.onExit(event => { exited = true; exitCode = event.exitCode; });
terminal.onData(data => child.write(data));
terminal.parser.registerCsiHandler({ final: 'c' }, params => {
  if (!params.length || (params.length === 1 && params[0] === 0)) {
    child.write('\x1b[?61;4c'); return true;
  }
  return false;
});
function lines() {
  const b = terminal.buffer.active;
  return Array.from({ length: b.length }, (_, row) => b.getLine(row).translateToString(true));
}
function check(label) {
  const text = lines();
  trace({ label, baseY: terminal.buffer.active.baseY, lines: text });
  for (const expected of ['ShellHistoryBeforeScode', 'SharedPartialJoined', 'SameBatch:end',
    'StdoutPrefix:StderrSuffix', ...Array.from({ length: 70 }, (_, i) => `Earlier history line ${i}`)]) {
    assert.equal(text.filter(line => line === expected).length, 1,
      `${label}: lost or duplicated ${expected}\n${text.join('\n')}`);
  }
  for (const marker of ['StatusMarker', 'TodoMarker', 'DraftSurvives', 'FooterMarker'])
    assert.equal(text.join('\n').split(marker).length - 1, 1, `${label}: ${marker}`);
  console.log(JSON.stringify({ mode, cursorMode, label, history: 70, frameCopies: 1, appRepaints: 0 }));
}
async function waitFor(predicate) {
  const until = Date.now() + 15000;
  while (Date.now() < until) {
    if (!pending && Date.now() - lastData >= 250 && predicate()) return;
    assert(!exited, 'fixture exited before acknowledgment');
    await sleep(25);
  }
  throw new Error('timed out waiting for static fixture');
}
async function stop() {
  if (!exited) child.write('\x1b');
  for (let i = 0; i < 250; i++) {
    if (exitCode === undefined) exitCode = child._agent?.exitCode;
    if (exited && exitCode !== undefined && !pending) return;
    await sleep(20);
  }
  if (!exited) child.kill();
  throw new Error('fixture did not exit cleanly');
}
(async () => {
  try {
    await waitFor(() => lines().some(line => line.includes('FooterMarker')));
    check('initial');
    let count = 0;
    for (const [cols, rows] of [[100,40], [240,40], [60,40], [240,40], [60,18], [240,40]]) {
      trace({ resize: [cols, rows] });
      terminal.resize(cols, rows);
      child.resize(cols, rows);
      child.write('\x1b[17~');
      count++;
      await waitFor(() => fs.existsSync(ack) && fs.readFileSync(ack, 'utf8').trim().split(/\r?\n/).at(-1) === String(count));
      check(`${cols}x${rows}`);
    }
    await stop();
    assert.equal(exitCode, 0, 'fixture exit');
  } finally {
    await stop();
    terminal.dispose();
  }
})().then(() => process.exit(0)).catch(error => { console.error(error); process.exit(1); });
