// One native diagnostic inventory for Unix and both Windows ConPTY backends.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');

const [fixture, directory] = process.argv.slice(2);
assert(fixture && directory);
fs.mkdirSync(directory, { recursive: true });
const failures = [];
for (const backend of process.platform === 'win32' ? ['bundled', 'system'] : ['bundled']) {
  for (const mode of ['fallback', 'handoff']) {
    for (const document of ['ascii', 'combining', 'laptop']) {
      for (const traced of [false, true]) {
        for (const delayedWrap of process.platform === 'win32' ? [false, true] : [false]) {
          const label = `${backend}-${mode}-${document}-${traced ? 'traced' : 'untraced'}-${delayedWrap ? 'delayed' : 'inherited'}`;
          const prefix = path.resolve(directory, label);
          const env = { ...process.env, IOCRAFT_UNICODE_HISTORY_CASE: document,
            IOCRAFT_UNICODE_HISTORY_MODE: mode, IOCRAFT_WIRE_TRACE: `${prefix}-wire.jsonl` };
          if (process.platform === 'win32') env.ELECTRON_RUN_AS_NODE = '1';
          delete env.IOCRAFT_DIAG_DELAYED_WRAP;
          if (delayedWrap) env.IOCRAFT_DIAG_DELAYED_WRAP = '1';
          delete env.IOCRAFT_FRAME_WRITE_TRACE;
          if (traced) env.IOCRAFT_FRAME_WRITE_TRACE = `${prefix}-application.log`;
          const stdout = fs.openSync(`${prefix}.stdout.log`, 'wx');
          const stderr = fs.openSync(`${prefix}.stderr.log`, 'wx');
          let result;
          try {
            result = spawnSync(process.env.IOCRAFT_TEST_NODE || process.execPath,
              [path.join(__dirname, 'unicode_history_pty.cjs'),
              path.resolve(fixture), backend], { env, stdio: ['ignore', stdout, stderr] });
          } finally {
            fs.closeSync(stdout);
            fs.closeSync(stderr);
          }
          console.log(JSON.stringify({ label, status: result.status, signal: result.signal,
            error: result.error?.message }));
          if (result.status !== 0 || result.error || result.signal) failures.push(label);
        }
      }
    }
  }
}
assert.deepEqual(failures, [], `Native Unicode history failures: ${failures.join(', ')}`);
