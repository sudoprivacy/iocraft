// Diagnostic, not a real-PTY acceptance test: isolate native frontend reflow
// before the application emits any erase or repaint. A stable anchor here is
// necessary, but not sufficient, for a safe ConPTY rendering protocol.
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const fromHost = name => require(path.join(process.env.IOCRAFT_TERMINAL_MODULES, name));
const { Terminal } = fromHost('@xterm/headless');
const write = (terminal, bytes) => new Promise(resolve => terminal.write(bytes, resolve));
const rows = ['StatusMarker ' + 'long status '.repeat(8), 'TodoMarker 1 todos (1 done, 0 open)',
  '─'.repeat(240), 'InputMarker DraftSurvives', '─'.repeat(240), 'FooterMarker'];
function locate(terminal) {
  const buffer = terminal.buffer.active;
  const lines = Array.from({ length: buffer.length }, (_, i) => buffer.getLine(i).translateToString(true));
  const statusCopies = lines.filter(line => line.includes('StatusMarker')).length;
  for (let index = 0; index < 70; index++) assert(lines.includes(`Earlier history line ${index}`));
  return { cursor: buffer.baseY + buffer.cursorY, status: lines.findIndex(line => line.includes('StatusMarker')), statusCopies,
    baseY: buffer.baseY };
}
(async () => {
  for (const mode of ['bundled', 'system']) {
    for (const anchor of ['first-row', 'blank-before', 'blank-after', 'saved-before']) {
      for (const wrap of [true, false]) {
        const terminal = new Terminal({ cols: 240, rows: 40, scrollback: 10000,
          allowProposedApi: true, reflowCursorLine: mode !== 'system',
          windowsPty: { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } });
        await write(terminal, Array.from({ length: 70 }, (_, i) => `Earlier history line ${i}\r\n`).join(''));
        await write(terminal, (anchor === 'saved-before' ? '\x1b7' : '')
          + (wrap ? '' : '\x1b[?7l') + (anchor === 'blank-before' ? '\r\n' : '')
          + rows.join('\r\n') + (anchor === 'blank-after' ? '\r\n' : '') + (wrap ? '' : '\x1b[?7h')
          + (anchor === 'first-row' ? '\x1b[5F' : anchor === 'blank-before' ? '\x1b[6F' : anchor === 'saved-before' ? '\x1b8' : '\r'));
        const stages = [{ width: 240, height: 40, ...locate(terminal) }];
        for (const [width, height] of [[100,40], [240,40], [60,40], [240,40], [60,18], [240,40]]) {
          terminal.resize(width, height);
          if (anchor === 'saved-before') await write(terminal, '\x1b8');
          stages.push({ width, height, ...locate(terminal) });
        }
        console.log(JSON.stringify({ mode, anchor, wrap, stages }));
        terminal.dispose();
      }
    }
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
