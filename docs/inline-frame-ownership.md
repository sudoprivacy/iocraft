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

Remaining before delivery: verified region recovery and non-destructive reporting
when old live rows are no longer addressable. The owner has approved input-first
small-window folding; its height budget and bounded review body are implemented
in sudocode draft #846, but are not yet merged or installed.

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

Pull requests and main pushes run `transactions` and `oversized-history`. Manual dispatch also accepts
`resize`, `rapid` and `tiny`; they retain their failing assertions and fail the job
when the defect is reproduced. A green transaction job establishes only the
history-transaction behavior above. Full resize acceptance remains required before
the renderer repair can be delivered.

To run the same wrapper locally, provide the pinned Code executable, its
`resources/app` directory, the newly built fixture executable and a dedicated log
directory to `packages/iocraft/tests/run_inline_history_pty.ps1`. Select an optional
`-Scenario resize`, `rapid` or `tiny` to run the corresponding diagnostic.

### Preserve history when the live frame exceeds the viewport

Closing a live panel taller than the window previously sent both display erase
and saved-line purge (`CSI 3 J`). That deleted shell history preceding iocraft.
The fallback now erases only the visible display. Real PTY verification opens a
60-line panel, closes it, and checks the startup sentinel, all 70 history lines,
three committed output lines, the draft, and a successful process exit.
Both ConPTY backends lost the sentinel before the change and preserved history
after it on Windows build 26300. There were still 26 stale panel rows in scrollback;
this test reports that count and does not establish full frame recovery.

The pinned node-pty can emit its public exit event on pipe close before its native
callback records an exit code. The driver waits for that native result when the
event carries no code; an absent result still fails. Local negative controls with
`IOCRAFT_FIXTURE_EXIT_CODE=7` failed on both backends with the actual code 7.

### Live-frame output batches

The terminal now stages live-frame bytes until the synchronized-update boundary.
A Canvas's internal flush no longer exposes an incomplete application frame.
History writes still use explicit barriers before stream switches and cursor
queries. The public component-clear API also retains its immediate-clear ordering
before external output; it has not silently become a deferred request.

Normal completion explicitly closes and flushes the update and returns failures.
The drop guard is only best-effort cleanup for early errors or panics, not the
normal error-reporting path. A failed partial write is not replayed on drop.
Four new regressions cover internal flush deferral, explicit-clear ordering,
final-flush error propagation, and closing on body errors/panics. This remains
a bounded current-output batch, not a transcript replay cache.

This is **not an atomic PTY/resize transaction**. The PR's top-anchor geometry
still fails the real bundled-ConPTY resize test. The buffer, history barriers
and error propagation are a separate implementation slice, not a resize waiver.

### Explicit recovery research, not deployed

The owner accepted preserving history and the draft, displaying one clear notice,
and continuing input when the old UI cannot be erased losslessly. Such recovery
must be reported as degraded, not as artifact-free success. Reachable ordinary
resize still has to clear the old live region.

A tail-anchor/native-cursor experiment recorded the old frame starting at absolute
row 74 while frontend resize raised the viewport start to 75, before requesting
the child resize. Its old frame needed 13 rows above a cursor reported at row 12.
That supports runtime detection in this sample, not an authoritative native
cursor guarantee for all hosts.

Combining the experimental policy with staged writes and stale-layout rejection
passed 10 bundled-ConPTY rapid/slow matrices without application or wire tracing,
including new typing and history after recovery. Adding history during resize at
1, 17 and 40 ms intervals passed 8 of 9 runs; one 17 ms run still duplicated visible
Status. System ConPTY still failed history preservation. The failed cases remain
failures; they are not silently accepted as the newly approved degradation.

The experiment is retained locally as `experiment/reflow-boundary-evidence`,
commit `189eb5a`. Its temporary worktree was removed; logs remain. The runtime
notice, cursor classification, tail anchor, timer, precommit layout retry and
experimental deferred-clear API are not included in this PR.

### Complete history shares the live-frame transaction (2026-10-03)

An additional regression first failed on `937fbf2`: complete history targeting
the render stream was flushed between clearing the old frame and painting its
replacement. Staging the Canvas alone did not keep that transaction together.
The presentation owner now treats its staged erase as the initial render-stream
segment. It flushes only at a stream switch, before a cursor query for an
unfinished line, or at the final frame boundary. An alternate stream still
flushes before returning to the live frame. Explicit component-clear semantics
are unchanged. This is one shared rule, not a history-source or slot exception.

The new regression checks both stdout and stderr as the render stream and
requires the destination to remain untouched until the complete frame commits.
All 189 local workspace tests, formatting and CI-equivalent strict Clippy pass.
This closes a deterministic transaction defect, not the outstanding resize gap.
The newly built fixture also passes transactions and oversized-history on both
bundled and system ConPTY (four normal exits with code 0). Oversized history still
reports 26 saved live rows. Strict resize still fails: bundled duplicates Status
at 100 columns; system loses history line 43 after returning to 240 columns.

### Stale geometry can erase reachable history (2026-10-03)

The rejected tail/native-cursor experiment has a stronger counterexample than
residue. A real bundled-ConPTY trace recorded a recovery plan sampled at 100
columns, needing an 11-row rewind. By the time its bytes reached xterm, the
frontend was 240 columns and only six rows separated the cursor and live start.
`CSI 11 F` followed by erase therefore reached into history. Repeating this race
lost numbered history even with no saved-line purge. A later successful footer
does not repair or excuse the lost history.

A deterministic xterm-only replay isolates that sequence: shrinking and growing
alone preserve every line; one complete synchronized clear/paint write then
removes history lines 68 and 69 plus the three committed history lines. It fails
the preservation assertion intentionally and is a diagnostic, not a passing
test or a replacement for real PTY coverage. It also proves that removing
mid-frame flushes alone cannot make stale geometry safe.

The research driver now collects traces in memory and dumps them at shutdown
to avoid synchronous file I/O in the race. Six traced 17 ms interleaving runs
on the old candidate gave three passes, two history-loss failures and one
visible-duplicate failure. Keeping same-stream history in the frame batch gave
four passes, one history-loss failure and one acknowledgment timeout in six
more runs. Load and instrumentation affect timing; these are counterexamples,
not statistically controlled performance comparisons. Neither candidate is
accepted or deployed. The runtime recovery notice is still experimental.

The next geometry design must account for a second owner: a frontend can resize
before the application's backend sees that size, or before queued output arrives.
A successful native cursor query and matching application-side sizes do not
establish that the erase will execute in that same geometry generation. Avoid
claiming an atomic resize transaction from batching or synchronized output:
[the synchronized-output protocol](https://contour-terminal.org/vt-extensions/synchronized-output/)
defers visible painting, while the emulator continues interpreting input.

### Mainline compatibility and external ownership (2026-10-06)

The research branch integrates main `2e495ed`, retaining span backgrounds,
terminal palette discovery, ordered input editing, Unix event readiness and
exclusive external terminal operations. Input editing uses main's implementation
unchanged. Its dedicated input PTY workflow replaces the duplicate input job in
the inline workflow; none of its six scenarios or two backends were removed.

External handoff is another presentation boundary. After the backend releases
the terminal, `PresentationState::release_terminal` invalidates both the canvas
baseline and partial-history continuation coordinates, while retaining queued
history. The render loop no longer owns a separate canvas reset. The same rule
applies when an external callback panics. A regression fails without the release
call: an identical resumed frame paints once rather than twice. The fixed test
also checks that resuming does not erase the external program's output.

A real-PTY `handoff` scenario now runs on both Windows backends in CI. It leaves
an unfinished history line, transfers stdin to a blocking external operation,
then resumes an identical canvas after output without a newline. Both local
backends preserved all 70 history lines, committed messages and the draft;
input returned to the UI and subsequent history did not overwrite external
output. All 12 input-editing scenarios passed. Oversized-panel history also
passed on both backends but still reported 26 saved live rows.

Strict resize acceptance remains failed after this integration: bundled ConPTY
duplicates Status on 240→100; system ConPTY loses history line 43 on 100→240.
These results still prohibit merging this PR as a completed resize repair or
replacing the installed CLI. Handoff during resize is not covered by the new
non-resize test, and inherits the unresolved geometry problem.

### Frontend-only anchor controls (2026-10-06)

`cursor_boundary_matrix.cjs` isolates reflow without application erase/repaint.
It compares the first UI row, a blank guard row before the UI, a blank tail, and
DECSC/DECRC saved positioning, with autowrap enabled and disabled while writing
the frame. It uses the same pinned xterm version and both host configurations.
This is a diagnostic, not a real-PTY or GUI acceptance test.

With the bundled configuration, width 240→100 moves a first-row cursor from
absolute row 70 to 75 while Status stays at 70. A blank guard's cursor also moves
70→75 while Status stays at 71. Saved positioning exhibits the same first-row
drift. Disabling autowrap while writing makes no difference in these samples.
The blank tail remains after the frame, but its distance changes from 6 to 11
rows, retaining the stale-width rewind hazard described above. Short numbered
history remains intact in the diagnostic; one guard-row height-shrink sequence
loses Status before any application erase. None establishes a safe runtime
anchor contract or expands the approved degradation policy.
