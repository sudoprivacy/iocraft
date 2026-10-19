// Real ConPTY diagnostic for controlled parent edits inside a key burst.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const assert = require('node:assert/strict');
const fromHost = name => require(path.join(process.env.IOCRAFT_TERMINAL_MODULES, name));
const { Terminal } = fromHost('@xterm/headless');
const pty = fromHost('node-pty');
const [fixture, mode, scenario] = process.argv.slice(2);
assert(fixture && ['bundled', 'system'].includes(mode) && ['clear', 'paste', 'middle-paste', 'clear-paste', 'navigation', 'rendered-cursor', 'resize-boundary', 'resize-restore-boundary', 'resize-narrow-burst'].includes(scenario));
const terminal = new Terminal({ cols: 100, rows: 30, scrollback: 1000, allowProposedApi: true,
  ...(process.platform === 'win32' ? { reflowCursorLine: mode !== 'system',
    windowsPty: { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } } : {}) });
const env = { ...process.env, TERM: 'xterm-256color' };
delete env.ELECTRON_RUN_AS_NODE;
delete env.NO_COLOR;
const child = pty.spawn(path.resolve(fixture), [], { cols: 100, rows: 30,
  cwd: path.dirname(path.resolve(fixture)), env, useConpty: true, useConptyDll: mode === 'bundled' });
let pending = 0, lastData = Date.now(), exited = false, exitCode;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function screen() {
  const b = terminal.buffer.active;
  return Array.from({ length: b.length }, (_, row) => b.getLine(row).translateToString(true)).join('\n');
}
function viewport() {
  const b = terminal.buffer.active;
  return Array.from({ length: terminal.rows }, (_, row) => b.getLine(b.baseY + row)?.translateToString(true) ?? '').join('\n');
}
child.onData(data => {
  if (process.env.IOCRAFT_WIRE_TRACE)
    fs.appendFileSync(process.env.IOCRAFT_WIRE_TRACE, JSON.stringify({ time: Date.now(), data }) + '\n');
  pending++; lastData = Date.now(); terminal.write(data, () => pending--);
});
child.onExit(event => { exited = true; exitCode = event.exitCode; });
terminal.onData(data => child.write(data));
terminal.parser.registerCsiHandler({ final: 'c' }, params => {
  if (!params.length || (params.length === 1 && params[0] === 0)) {
    child.write('\x1b[?61;4c'); return true;
  }
  return false;
});
async function waitFor(text, settled = true) {
  const until = Date.now() + 15000;
  while (Date.now() < until) {
    if (!pending && (!settled || Date.now() - lastData > 250) && viewport().includes(text)) return;
    assert(!exited, `fixture exited before ${text}`);
    await sleep(20);
  }
  throw new Error(`missing ${text}\n${screen()}`);
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
    await waitFor('InputValue::EndValue');
    const narrowBurst = scenario === 'resize-narrow-burst';
    const seed = narrowBurst ? 'DraftHeadTail' : 'Seed';
    child.write(seed);
    await waitFor(`InputValue:${seed}:EndValue`, !narrowBurst);
    if (narrowBurst) {
      // Match the real CLI's middle-cursor preparation. Observe rendered text,
      // but do not insert a quiet-period barrier before the critical resize.
      child.write('\x1b[H\x1b[C\x1b[C\x1b[C\x1b[C\x1b[CX');
      await waitFor('InputValue:DraftXHeadTail:EndValue', false);
      child.write('\x7f');
      await waitFor('InputValue:DraftHeadTail:EndValue', false);
    }
    if (scenario === 'resize-boundary' || scenario === 'resize-restore-boundary' || scenario === 'resize-narrow-burst') {
      const narrow = scenario === 'resize-narrow-burst';
      const width = narrow ? 12 : 30;
      terminal.resize(width, 8);
      child.resize(width, 8);
      if (scenario === 'resize-restore-boundary') await waitFor('Viewport:30x8:blocked');
      // F6 acknowledges delivery. For the restore case do not wait between
      // hidden input and growth: those pre-resize keys still own the old frame.
      child.write('MustNotAppear' + (narrow ? '' : '\r\x1b[17~'));
      if (narrow) await waitFor('Focus:off');
      if (scenario === 'resize-boundary') {
        await waitFor('Viewport:30x8:blocked');
        await waitFor('BatchAck:1');
      }
      terminal.resize(100, 30);
      child.resize(100, 30);
      // A restored size marker may also exist in scrollback from startup.
      // Require a fresh, non-editing acknowledgment in the actual viewport;
      // the critical resize/input burst above remains unsynchronized.
      const restoredAck = narrow ? 1 : 2;
      child.write('\x1b[17~');
      await waitFor('Viewport:100x30:reviewable');
      await waitFor(`BatchAck:${restoredAck}`);
      const latestValue = () => [...viewport().matchAll(/InputValue:([^\r\n]*):EndValue/g)].at(-1)?.[1];
      const latestConfirmed = () => [...viewport().matchAll(/Confirmed:(\d+)/g)].at(-1)?.[1];
      assert.equal(latestValue(), seed, `${mode}: hidden draft mutated\n${screen()}`);
      assert.equal(latestConfirmed(), '0', `${mode}: unseen controls confirmed\n${screen()}`);
      child.write('Kept\r\x1b[17~');
      await waitFor(`BatchAck:${restoredAck + 1}`);
      const expected = narrow ? 'DraftKeptHeadTail' : 'SeedKept';
      assert.equal(latestValue(), expected, 'focus restored after resize');
      assert.equal(latestConfirmed(), '1', 'visible controls remain operable');
      console.log(JSON.stringify({ mode, scenario, expected, result: 'PASS' }));
      await stop();
      assert.equal(exitCode, 0, 'fixture exit');
      return;
    }
    const cases = {
      clear: ['\x15Fresh', 'Fresh'],
      paste: ['\x1b[200~Pasted\x1b[201~Fresh', 'Seed[Pasted]Fresh'],
      'middle-paste': ['\x1b[H\x1b[C\x1b[C\x1b[200~界\x1b[201~Fresh', 'Se[界]Freshed'],
      'clear-paste': ['\x15\x1b[200~Pasted\x1b[201~Fresh', '[Pasted]Fresh'],
      navigation: ['\x1b[H\x1b[CX\x1b[D\x1b[3~\x1b[F\x7f\x1b[HZ', 'ZSee'],
      'rendered-cursor': ['\x1b[H\x1b[CX\x1b[D', 'SXeed'],
    };
    const [burst, expected] = cases[scenario];
    child.write(burst + '\x1b[17~');
    await waitFor('BatchAck:1');
    assert(screen().includes(`InputValue:${expected}:EndValue`), `${mode} ${scenario}: ${screen()}`);
    if (scenario === 'rendered-cursor') {
      child.write('Z\x1b[17~');
      await waitFor('BatchAck:2');
      assert(screen().includes('InputValue:SZXeed:EndValue'), `${mode} cursor after render: ${screen()}`);
    }
    console.log(JSON.stringify({ mode, scenario, expected, result: 'PASS' }));
    await stop();
    assert.equal(exitCode, 0, 'fixture exit');
  } finally { await stop(); terminal.dispose(); }
})().then(() => process.exit(0)).catch(error => { console.error(error); process.exit(1); });
