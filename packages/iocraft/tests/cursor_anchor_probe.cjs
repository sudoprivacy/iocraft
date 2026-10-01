// Diagnostic only: can a saved cursor delimit a live frame across reflow?
const path = require('node:path');
const modules = process.env.IOCRAFT_TERMINAL_MODULES;
const { Terminal } = require(path.join(modules, '@xterm/headless'));
const write = (term, value) => new Promise(resolve => term.write(value, resolve));

async function main() {
  for (const reflowCursorLine of [true, false]) {
    for (const [columns, rows] of [[100, 40], [60, 40], [240, 40], [30, 8]]) {
      const term = new Terminal({ cols: 240, rows: 40, scrollback: 1000,
        allowProposedApi: true, reflowCursorLine,
        windowsPty: { backend: 'conpty', buildNumber: 26200 } });
      for (let i = 0; i < 70; i++) await write(term, `History${i}\r\n`);
      await write(term, '\x1b7');
      await write(term, `StatusMarker ${'S'.repeat(110)}\r\nTodoMarker\r\n${'-'.repeat(240)}\r\nInputMarker\r\n${'-'.repeat(240)}\r\nFooterMarker\r\n`);
      term.resize(columns, rows);
      const buffer = term.buffer.active;
      let statusRow = -999;
      for (let i = 0; i < buffer.length; i++) {
        if (buffer.getLine(i).translateToString(true).includes('StatusMarker')) statusRow = i - buffer.baseY;
      }
      const tail = buffer.cursorY;
      await write(term, '\x1b8');
      console.log(JSON.stringify({ reflowCursorLine, columns, rows, statusRow, tail,
        restoredRow: buffer.cursorY, savedAnchorMatches: buffer.cursorY === statusRow }));
      term.dispose();
    }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
