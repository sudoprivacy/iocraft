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

### Shared frame dimensions and rapid-resize follow-up

Further tracing found a second ownership defect: after coalescing several resize
notifications, the layout engine used 240 columns while `use_terminal_size`
still returned 100 columns from its independently queued event state. The frame
therefore contained 100-column separators on a 240-column Canvas. This does not
depend on the Todo or Status implementation.

The renderer now supplies one immutable `TerminalSizeSnapshot` to the component
tree. All size hooks read that sample, which is also used for layout. Resize
events still wake components; their stored dimensions are only a fallback when
no terminal snapshot exists. The context is separate from `SystemContext`, so
holding a mutable system-context borrow cannot make the dimensions unavailable.
Mock terminals now update their geometry when emitting a resize, allowing tests
to assert both the hook's value and the Canvas width. There is no added timer,
new dependency, slot ordering change or per-keystroke history replay in this slice.

Local validation: 179 workspace tests pass, including snapshot changes without
intervening events and sibling hooks with a borrowed `SystemContext`. Formatting,
CI-equivalent strict Clippy and warnings-as-errors documentation build pass.

The independent tail-anchor experiment completed 50 additional slow bundled-
ConPTY runs (20 with application tracing and 30 without application tracing,
both with wire tracing), but still failed rapid resizing. A 35 ms resize-only
coalescing experiment avoided the immediate rapid-case duplication, yet a
subsequent slow resize to 60x40 exposed a duplicate Status row. Adding the shared
size snapshot and a sticky observed-size invalidation flag did not eliminate
that later failure. None of that tail, timer or invalidation experiment is part
of this PR. Earlier missing-Footer reports need revalidation with the complete-
frame acknowledgment below; they cannot independently establish a renderer loss.

Raw VT traces show a 100-column frame reaching the frontend after it was already
resized to 60 columns. Thus application size coherence is necessary but does not
establish a reliable post-reflow boundary by itself. Windows' own ConPTY cursor
resynchronization work also acknowledges that frontend and backend reflow can
diverge: [Microsoft Terminal #19535](https://github.com/microsoft/terminal/pull/19535).
That upstream change is not a substitute for our acceptance tests.

The PTY driver now accepts `rapid` and `tiny` as well as `resize`. After resizing,
F6 requests a new fixture acknowledgment containing the observed dimensions;
checks wait for that acknowledgment, not only a width-matching separator. Count
all Status markers, including stale phases. The rapid case includes a subsequent
slow matrix to expose latent corruption. `IOCRAFT_WIRE_TRACE` optionally records
the fixture's bytes and resize requests for diagnosis. These resize scenarios
remain failing acceptance diagnostics, not expected-failure tests declared green.

The same completeness rule now applies to non-resize history transactions:
wait for the new phase in the Footer, which is painted last. During concurrent
builds a 250 ms quiet interval occurred mid-frame and the old driver reported
missing Footer before the rest of the bytes arrived. A startup timeout was also
recorded in that run. Retain those logs, but do not confuse an incomplete sample
with verified content loss. The rapid-then-slow Status duplication above was
reproduced after receiving the matching new Footer acknowledgment.

Remaining before delivery: verified region recovery, an explicit layout height
budget, removal of the existing oversized-canvas scrollback-purge fallback, and
non-destructive reporting when old live rows are no longer addressable. The
small-window folding/scrolling policy still needs the application's decision.

### Real PTY CI coverage

`Inline PTY` builds `inline_history_fixture` and runs the history transaction
workflow on Windows with both bundled and system ConPTY. It checks partial-line
continuation across hooks, output ordering across streams, the input draft and
all 70 pre-application history lines. Every run retains stdout, stderr and the
raw terminal bytes as an artifact.

The host is VS Code 1.140.0, commit
`07f806f999227108933c2e30515b26eecc1fda74`, downloaded from Microsoft's versioned
update endpoint and verified against the archive SHA256. The same package supplies
Electron, node-pty and xterm. The driver reports those versions and uses the actual
Windows build number rather than a hardcoded development-machine build.

Local verification of the CI runner on Windows build 26300 passed the transaction
workflow on both backends. Explicit `resize` runs failed on both backends and the
wrapper returned a nonzero exit code while preserving each backend's diagnostics.
This confirms that the runner does not hide a failed acceptance scenario.

Pull requests and main pushes run `transactions`. Manual dispatch also accepts
`resize`, `rapid` and `tiny`; they retain their failing assertions and fail the job
when the defect is reproduced. A green transaction job establishes only the
history-transaction behavior above. Full resize acceptance remains required before
the renderer repair can be delivered.

To run the same wrapper locally, provide the pinned Code executable, its
`resources/app` directory, the newly built fixture executable and a dedicated log
directory to `packages/iocraft/tests/run_inline_history_pty.ps1`. Select an optional
`-Scenario resize`, `rapid` or `tiny` to run the corresponding diagnostic.
