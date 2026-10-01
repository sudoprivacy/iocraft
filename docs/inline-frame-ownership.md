# Inline frame ownership

Status: implementation in progress. This does **not** declare the resize defect
fixed. Follow-up to #4, whose start-cursor assumption fails with terminal reflow.

## Contract

The component tree supplies content. Layout allocates space. The terminal layer
owns the last presented frame, history writes, cursor state and recovery. A slot
must not erase or deduplicate another slot's output.

Keep normal-screen scrollback, including output preceding application startup.
Do not purge saved lines to make the interface look correct. Saving an agent
session is not a backup of the surrounding terminal. Bottom docking is a layout
choice, not a prerequisite for correctness.

## First implementation slice

Move the previous Canvas out of the component render loop. `Terminal` owns one
presentation state: the previous frame, pending history messages, and the
temporary newline used to separate unfinished history from the live frame.
`UseOutput` hooks only enqueue messages; they do not move the cursor or retain
their own newline/column state. Drain all hooks once per frame, clear the live
frame once, commit history, then present the next frame. Clear and resize
invalidation must update the same cached frame. Propagate write failures to the
render loop instead of swallowing them and advancing the presentation state.

This retains the existing traversal order between independently queued hooks;
it does not claim a new global enqueue-order guarantee. Buffered stdout/stderr
writes must be flushed before switching streams. Ordinary unchanged frames keep
row diffing; no history replay or additional unbounded history cache is added.

The backend still translates frame operations to terminal coordinates. The next
slice must consolidate its geometry into an explicit valid/invalid region state,
rather than treating an old cursor row as proof of ownership after resize.

## Resize evidence and remaining work

- The framework-only fixture reproduces repeated live UI without an agent engine.
- VS Code's bundled ConPTY and the system ConPTY do not behave identically. A
  tail-anchor/reflow-height prototype passed one backend but erased history on
  another. It is not an acceptable implementation.
- Saving/restoring a cursor does not establish a logical anchor: with VS Code
  1.140.0's xterm model, a 240-column frame resized to 100 columns moved its start
  to viewport row 28 while the restored cursor was still on row 33.
- A 120x16 to 30x8 protocol diagnostic moves old live content to row -11, outside
  cursor-addressable space. Display erase cannot reach it; saved-line purge also
  deletes pre-application history. Scroll margins did not fix that diagnostic.
  These are specific model observations, not a universal impossibility claim.

Next: represent size generations and owned bounds; invalidate diff coordinates
on resize; recover only from verified geometry. Distinguish reachable live rows
from inaccessible scrollback and report degradation without destructive cleanup.
The application layout needs an explicit height budget that keeps input usable
without silently hiding other functionality. Do not change slot order as part of
this work.

## Validation

Verify actual emitted bytes through a real PTY and a terminal model retaining
scrollback, not just the final visible viewport. Keep every seeded history line,
the input draft and selection state. Check both directions of repeated width and
height changes, rapid resize, small windows and interleaved history/tool output.
Record backend and terminal versions. A clean visible viewport with lost history
or stale UI in scrollback is a failure, not a pass.

The first slice additionally checks output from separate hooks, partial-line
continuation across frames, cross-stream order, unchanged-frame diffing and
propagation of write/flush failures. Passing those checks does not waive the
remaining resize acceptance matrix.

### Local results, 2026-10-02

Windows 11 build 26200; VS Code 1.140.0's bundled node-pty and xterm modules;
Rust 1.93.1. The same fixture on baseline `7cc7f98` fails at the independent
hook continuation: `SharedPartial` and `Joined` appear on separate lines and
the live frame is corrupted. This slice passes all non-resize stages with
both bundled and system ConPTY, retaining all 70 seeded lines, the startup
sentinel and the draft.

The optional resize stage is **still failing**. A stricter driver waits for new
backend output and an exact new-width separator, not just an already-visible
status marker; it catches duplication at the first 240x40 to 100x40 resize.
An earlier, weaker settle check did not fail until 60x18. Those intermediate
apparent passes are not valid evidence of resize recovery. Do not install or describe this slice as the
resize fix. `--all-targets --all-features` Clippy also reports 18 pre-existing
test-code lint errors on both the baseline and this branch; no suppressions
were added. CI-equivalent checks are reported separately.

Reproduce with the same terminal modules (not arbitrary globally installed
packages): build `cargo build -p iocraft --example inline_history_fixture`, set
`IOCRAFT_TERMINAL_MODULES` to the host's module directory, then run
`packages/iocraft/tests/inline_history_pty.cjs <fixture-path> bundled` or
`system`. Append `resize` to include the known failing matrix. For VS Code's
asar/native modules, use that installation's Code executable with
`ELECTRON_RUN_AS_NODE=1`. These diagnostic runs do not touch the user's terminal
history or start an agent/model session.

### Recovery experiments (not included in the implementation)

The tail-only candidate was retested without the earlier bottom-docking code.
It still failed: missing Footer in bundled ConPTY after repeated width changes,
and missing history in system ConPTY. Do not attribute the earlier failures to
bottom docking alone or merge the tail candidate.

A static control writes 70 numbered history lines and a six-row frame once,
then does **no redraw on resize**. With system ConPTY / xterm in the tested
configuration, 240x40 -> 100x40 -> 240x40 loses history lines 37..41; the next
60-column round trip additionally loses 42..44. Bundled ConPTY retains all 70.
This is evidence of a backend/model interaction independent of our redraw, not
permission to weaken preservation checks or a universal claim about Windows.

The harness mirrors VS Code 1.140.0's `reflowCursorLine` setting and its ConPTY
primary device-attributes reply (`CSI ? 61 ; 4 c`). The static control was rerun
with that reply and reproduced the same loss. The native Windows screen-buffer
view is also not always the frontend's view: with bundled ConPTY after widening,
the native Status row remains 28 while xterm displays it at 33. A Windows cursor
or screen-read query alone is therefore not an authoritative frontend boundary.
