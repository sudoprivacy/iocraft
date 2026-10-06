// Real ConPTY diagnostic for controlled parent edits inside a key burst.
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const assert = require('node:assert/strict');
const fromHost = name => require(path.join(process.env.IOCRAFT_TERMINAL_MODULES, name));
const { Terminal } = fromHost('@xterm/headless');
const pty = fromHost('node-pty');
const [fixture, mode, scenario] = process.argv.slice(2);
assert(fixture && ['bundled', 'system'].includes(mode) && ['clear', 'paste', 'middle-paste', 'clear-paste', 'navigation', 'rendered-cursor'].includes(scenario));
const terminal = new Terminal({ cols: 100, rows: 30, scrollback: 1000, allowProposedApi: true,
  reflowCursorLine: mode !== 'system', windowsPty: { backend: 'conpty', buildNumber: Number(os.release().split('.')[2]) } });
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
async function waitFor(text) {
  const until = Date.now() + 15000;
  while (Date.now() < until) {
    if (!pending && Date.now() - lastData > 250 && screen().includes(text)) return;
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
    child.write('Seed');
    await waitFor('InputValue:Seed:EndValue');
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
