// A terminal-protocol diagnostic, not a passing renderer regression test.
// Use the same @xterm/headless build as the terminal under investigation:
//   IOCRAFT_TERMINAL_MODULES=<node_modules or node_modules.asar>
// It performs no operations on the user's real terminal.
const path = require('node:path');
const assert = require('node:assert/strict');
const modules = process.env.IOCRAFT_TERMINAL_MODULES;
const { Terminal } = require(modules ? path.join(modules, '@xterm/headless') : '@xterm/headless');

const write = (term, text) => new Promise(resolve => term.write(text, resolve));
function snapshot(term) {
  const buffer = term.buffer.active;
  return Array.from({ length: buffer.length }, (_, i) => ({
    absoluteRow: i,
    viewportRow: i - buffer.baseY,
    text: buffer.getLine(i).translateToString(true),
  }));
}

async function main() {
  for (const scrollMargins of [false, true]) {
    const term = new Terminal({
      cols: 120, rows: 16, scrollback: 1000, allowProposedApi: true,
      reflowCursorLine: true, windowsPty: { backend: 'conpty', buildNumber: 26200 },
    });
    try {
      await write(term, 'ShellHistoryBeforeScode\r\n');
      for (let i = 0; i < 24; i++) await write(term, `History${i}\r\n`);
      // Model a live frame that fits easily BEFORE the resize, followed by
      // a blank cursor guard row. Include exact-width separator rows.
      await write(term, `OldStatusMarker ${'S'.repeat(95)}\r\nOldTodoMarker ${'T'.repeat(95)}\r\n`);
      await write(term, `${'-'.repeat(120)}\r\nInputDraft\r\n${'-'.repeat(120)}\r\nFooter\r\n`);
      if (scrollMargins) await write(term, '\x1b[1;9r\x1b[16;1H');
      term.resize(30, 8);
      const beforeClear = snapshot(term);
      const oldStatus = beforeClear.find(line => line.text.includes('OldStatusMarker'));
      assert(oldStatus && oldStatus.viewportRow < 0, 'old live frame must have entered scrollback');
      // Even a full visible-screen erase cannot reach the old frame now.
      await write(term, '\x1b[r\x1b[H\x1b[2J');
      const afterDisplayErase = snapshot(term);
      assert(afterDisplayErase.some(line => line.text.includes('OldStatusMarker')));
      assert(afterDisplayErase.some(line => line.text.includes('ShellHistoryBeforeScode')));
      // Purging saved lines reaches it, but also destroys pre-application history.
      await write(term, '\x1b[3J');
      const afterPurge = snapshot(term);
      assert(!afterPurge.some(line => line.text.includes('OldStatusMarker')));
      assert(!afterPurge.some(line => line.text.includes('ShellHistoryBeforeScode')));
      console.log(JSON.stringify({
        scrollMargins, oldStatusViewportRow: oldStatus.viewportRow,
        displayEraseLeavesOldUi: true, savedLinesPurgeAlsoLosesShellHistory: true,
      }));
    } finally { term.dispose(); }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
