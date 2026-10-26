// Real-PTY history transaction regression. On Windows this may run with
// ELECTRON_RUN_AS_NODE=1 using VS Code's matching node-pty/xterm modules.
// IOCRAFT_TERMINAL_MODULES points at node_modules (or node_modules.asar).
// Usage: node inline_history_pty.cjs <fixture.exe> [bundled|system] [resize|rapid|tiny|oversized-history]
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
assert(!resize || ['resize', 'rapid', 'interleaved', 'tiny', 'bounded-tiny', 'queued-frame', 'history-growth', 'row-shift', 'oversized-history'].includes(resize), `unknown scenario: ${resize}`);
const wireTrace = [];
const trace = record => {
  if (process.env.IOCRAFT_WIRE_TRACE) wireTrace.push({ time: Date.now(), ...record });
};
const initialRows = resize === 'history-growth' ? 120 : 40;
const historyGateRoot = resize === 'history-growth'
  ? fs.mkdtempSync(path.join(os.tmpdir(), 'iocraft-history-growth-')) : undefined;
const terminal = new Terminal({ cols: 240, rows: initialRows, scrollback: 10000,
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
if (historyGateRoot) env.IOCRAFT_FIXTURE_HISTORY_GATE = path.join(historyGateRoot, 'release');
const child = pty.spawn(path.resolve(fixture), [], { cols: 240, rows: initialRows,
  cwd: path.dirname(path.resolve(fixture)), env, useConpty: true, useConptyDll: mode === 'bundled' });
let pending = 0, lastData = Date.now(), exited = false;
let exitCode;
let frames = 0;
const presentationObservers = new Set();
terminal.parser.registerCsiHandler({ prefix: '?', final: 'l' }, params => {
  if (params.includes(2026)) {
    frames++;
    for (const observer of presentationObservers) observer();
  }
  return false;
});
let lastResize;
let queuedFrameResize;
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
  if (queuedFrameResize && data.includes('\x1b[?2026h')) {
    const [cols, rows] = queuedFrameResize;
    queuedFrameResize = undefined;
    // The child has already emitted its frame using the old dimensions.
    // Resize before the frontend parses it, closing the race left by even
    // the last OS-size preflight without relying on scheduler timing.
    trace({ resize: [cols, rows], queuedFrame: true });
    terminal.resize(cols, rows);
    child.resize(cols, rows);
  }
  trace({ data });
  lastData = Date.now(); pending++;
  terminal.write(data, () => pending--);
});
terminal.onData(data => { trace({ terminalReply: data }); child.write(data); });
// Match VS Code's ConPTY device-attributes reply, not xterm's default DA.
if (process.platform === 'win32') terminal.parser.registerCsiHandler({ final: 'c' }, params => {
  if (!params.length || (params.length === 1 && params[0] === 0)) {
    child.write('\x1b[?61;4c'); return true;
  }
  return false;
});
child.onExit(event => { exited = true; exitCode = event.exitCode; });
async function settle(marker, width, notBefore = 0, afterFrame = -1) {
  const start = Date.now();
  while (Date.now() - start < 15000) {
    await sleep(25);
    const text = snapshot();
    if (!pending && frames > afterFrame && lastData >= notBefore && Date.now() - lastData >= 250 && text.includes(marker)
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
    await settle(`FooterMarker p0 a0 240x${initialRows}`);
    child.write('DraftSurvives');
    await settle('DraftSurvives');
    check('initial', 0, []);
    child.write('\x1bOQ'); // F2, first hook leaves an unfinished line.
    await settle(`FooterMarker p2 a0 240x${initialRows}`);
    check('partial', 2, ['SharedPartial']);
    child.write('\x1bOR'); // F3, independent hook must continue that line.
    await settle(`FooterMarker p3 a0 240x${initialRows}`);
    check('cross-hook continuation', 3, ['SharedPartialJoined']);
    child.write('\x1bOS'); // F4, two hook batches in one frame.
    await settle(`FooterMarker p4 a0 240x${initialRows}`);
    check('same-frame batches', 4, ['SharedPartialJoined', 'SameBatch:end']);
    child.write('\x1b[15~'); // F5, buffered stdout followed by stderr.
    await settle(`FooterMarker p5 a0 240x${initialRows}`);
    const committed = ['SharedPartialJoined', 'SameBatch:end', 'StdoutPrefix:StderrSuffix'];
    check('cross-stream order', 5, committed);
    if (resize === 'queued-frame') {
      terminal.resize(60, 40); child.resize(60, 40);
      await settle('FooterMarker p5 a0 60x40', 60);
      check('before queued frame', 5, committed);
      queuedFrameResize = [40, 12];
      child.write('\x1bOP'); // F1 changes the first and last live rows.
      await settle('FooterMarker p1 a0 40x12', 40);
      assert.equal(queuedFrameResize, undefined, 'the old-size frame was not intercepted');
      check('queued frame during shrink', 1, committed);
      terminal.resize(240, 40); child.resize(240, 40);
      await settle('FooterMarker p1 a0 240x40', 240);
      check('growth after queued frame', 1, committed);
    } else if (resize === 'history-growth') {
      const before = frames, started = Date.now();
      let firstGrowthFrame;
      const observe = () => {
        const text = snapshot();
        if (firstGrowthFrame === undefined && text.includes('FooterMarker p5 a0 240x160'))
          firstGrowthFrame = text;
      };
      presentationObservers.add(observe);
      trace({ resize: [240,160] });
      terminal.resize(240,160); child.resize(240,160);
      fs.writeFileSync(env.IOCRAFT_FIXTURE_HISTORY_GATE, 'release');
      await settle('FooterMarker p5 a0 240x160', 240, started, before);
      check('history during growth', 5, [...committed, 'ConcurrentFirst', 'ConcurrentSecond']);
      presentationObservers.delete(observe);
      assert(firstGrowthFrame?.includes('ConcurrentFirst') && firstGrowthFrame?.includes('ConcurrentSecond'),
        'history must join the first resized presentation');
      const lines = snapshot().split('\n');
      const lastBody = lines.findIndex(row => row.trim() === 'ConcurrentSecond');
      const status = lines.findIndex(row => row.includes('StatusMarker'));
      assert.equal(status, lastBody + 2, 'history must remain adjacent to live chrome');
    } else if (resize === 'row-shift') {
      for (const [key, tail] of [['\x1b[21~', 'ProseZeta'], ['\x1b[23~', 'ProseTheta'], ['\x1b[24~', 'ProseKappa']]) {
        const before = frames, started = Date.now();
        child.write(key);
        await settle(tail, 240, started, before);
        check(`prose ${tail}`, 5, committed);
        const text = snapshot();
        for (const marker of ['ProseGamma', 'ProseDelta', 'ProseEpsilon', 'ProseZeta'])
          assert.equal(text.split(marker).length - 1, 1, text);
        if (tail === 'ProseKappa') {
          assert(!text.includes('ProseAlpha') && !text.includes('ProseBeta'),
            'discarded live rows must not enter native scrollback');
          for (const marker of ['ProseEta', 'ProseTheta', 'ProseIota', 'ProseKappa'])
            assert.equal(text.split(marker).length - 1, 1, text);
        }
      }
    } else if (resize === 'oversized-history') {
      // A large live panel can exceed the viewport without any terminal resize.
      // Closing it must preserve shell history and earlier committed output.
      // This data-retention check does not waive the strict resize matrix above.
      child.write('\x1b[18~'); // F7: expand beyond the 40-row viewport.
      await settle('FooterMarker p7 a0 240x40', 240);
      assert(snapshot().includes('Expanded live content'), 'large panel did not open');
      child.write('\x1b[19~'); // F8: close the panel, exercising offscreen recovery.
      await settle('FooterMarker p8 a0 240x40', 240);
      const lines = snapshot().split('\n');
      for (const expected of ['ShellHistoryBeforeScode', ...committed,
        ...Array.from({ length: 70 }, (_, i) => `Earlier history line ${i}`)]) {
        assert.equal(lines.filter(line => line === expected).length, 1,
          `closing oversized panel lost or duplicated history: ${expected}\n${snapshot()}`);
      }
      assert(snapshot().includes('DraftSurvives'), 'closing panel lost the draft');
      console.log(JSON.stringify({ mode, label: 'oversized panel history', historyLines: 70,
        committed, staleLiveRows: lines.filter(line => line === 'Expanded live content').length }));
    } else if (['resize', 'rapid', 'interleaved', 'tiny', 'bounded-tiny'].includes(resize)) {
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
      if (resize === 'rapid' || resize === 'interleaved') {
        const interval = Number(process.env.IOCRAFT_RESIZE_INTERVAL_MS || 5);
        assert(Number.isFinite(interval) && interval >= 0, 'invalid resize interval');
        for (const [cols, rows] of matrix) {
          resizeTo(cols, rows);
          if (resize === 'interleaved') {
            child.write('\x1b[20~'); // F9: commit real output while resizing.
            committed.push(`ResizeHistory ${committed.length - 2}`);
          }
          await sleep(interval);
        }
        await acknowledge(240, 40);
        check('rapid resize', 5, committed);
      }
      // A subsequent slow round trip also detects latent corruption left by
      // a rapid round trip whose final dimensions matched its initial ones.
      for (const [cols, rows] of resize === 'tiny' ? [[30,8], [240,40]]
        : resize === 'bounded-tiny' ? [[60,40], [40,12], [120,40], [48,40], [240,40]] : matrix) {
        resizeTo(cols, rows);
        await acknowledge(cols, rows);
        check(`resize ${cols}x${rows}`, 5, committed);
      }
    }
    else if (resize) assert.fail(`scenario ${resize} has no acceptance workflow`);
    child.write('\x1b');
    for (let i = 0; i < 250; i++) {
      // With the pinned node-pty on Windows, pipe close can emit onExit before
      // the native process-exit callback stores its code. Read that eventual
      // result rather than treating a closed pipe as successful process exit.
      if (exitCode === undefined && process.platform === 'win32')
        exitCode = child._agent?.exitCode;
      if (exited && exitCode !== undefined && !pending) break;
      await sleep(20);
    }
    assert(exited, 'fixture failed to exit');
    assert.equal(exitCode, 0, 'fixture exited unsuccessfully');
    console.log(JSON.stringify({ mode, label: 'normal exit', exitCode }));
  } finally {
    // Give the owned fixture a normal exit even when an assertion fails;
    // abruptly closing a live ConPTY can hang native-host teardown.
    if (!exited) {
      child.write('\x1b');
      for (let i = 0; i < 100 && !exited; i++) await sleep(20);
    }
    if (!exited) child.kill();
    terminal.dispose();
    if (historyGateRoot) fs.rmSync(historyGateRoot, { recursive: true, force: true });
    // Synchronous disk writes between frontend and PTY resize artificially
    // separate their geometry changes, especially across Docker bind mounts.
    // Keep the original timestamps and write the complete trace after the run.
    if (process.env.IOCRAFT_WIRE_TRACE)
      fs.writeFileSync(process.env.IOCRAFT_WIRE_TRACE, wireTrace.map(row => JSON.stringify(row)).join('\n') + '\n');
  }
})().then(() => process.exit(0)).catch(error => { console.error(error); process.exit(1); });
