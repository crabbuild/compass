# Adaptive Compass Watch Design

**Status:** Approved  
**Date:** 2026-07-24  
**Scope:** Native `compass watch`; Graphify compatibility behavior remains frozen

## Problem

Compass already uses native filesystem events by default and falls back to
content-aware polling when users pass `--poll`. The current watcher filters
irrelevant paths and coalesces changes, but it uses a fixed three-second
debounce and invokes the cached build pipeline only after that delay.

The result is correct but feels slower than necessary during an editor session.
The watcher also exits on some infrastructure failures, does not synchronize
the graph when it starts, and provides limited feedback while work is queued or
running. A burst of events received during a build is not represented by an
explicit scheduling policy.

## Goals

- Make native filesystem-event watching the fast, reliable default.
- Start isolated builds after a short quiet period without allowing continuous
  edits to postpone builds indefinitely.
- Run at most one build at a time and coalesce events received during a build
  into one follow-up.
- Synchronize missing or stale graph artifacts when watch mode starts.
- Recover from transient watcher and build failures without requiring users to
  restart the process.
- Keep terminal output concise, actionable, and useful both interactively and
  in redirected logs.
- Preserve configured scope, ignore, safety, and atomic-publication behavior.
- Retain the ordinary incremental build as the correctness oracle.
- Keep scheduler boundaries suitable for future file-level graph patching.

## Non-goals

- Directly patching individual graph nodes or edges in this release.
- Publishing partially clustered or mixed-generation artifact bundles.
- Running semantic provider extraction automatically.
- Adding a persistent background daemon or operating-system service.
- Changing Graphify compatibility output, default timing, or macOS polling.
- Guaranteeing identical native filesystem event sequences across platforms.

## Chosen approach

This release adds a correctness-first adaptive scheduler around the existing
cached incremental build. Native events provide prompt invalidation, while the
normal build pipeline remains responsible for detection, extraction, graph
construction, clustering, rendering, completeness checks, and atomic
publication.

This approach improves perceived latency and operational resilience without
introducing a second graph mutation model. A future file-delta builder can
implement the same scheduler-facing build interface.

Alternatives considered:

1. A two-speed build that publishes structural output before clustering and
   visualization. This improves perceived speed but risks exposing artifacts
   from different generations.
2. Immediate file-level graph patching. This offers the lowest eventual
   latency but requires separate correctness work for renames, deletions,
   cross-file resolution, clustering, and atomic publication.

## Architecture

### Responsibilities

The enhanced watch path is divided into four units:

1. **Event source:** owns the native or polling `notify` watcher and sends raw
   events to the process.
2. **Event filter:** applies project root, saved scope, Git ignore, explicit
   exclusion, generated-output, sensitive-file, and program-artifact rules.
3. **Adaptive scheduler:** normalizes paths, coalesces events, tracks timing and
   retry state, and decides when one build may run.
4. **Build executor:** invokes the existing cached incremental build and reports
   success or failure without owning scheduling policy.

The scheduler does not inspect or mutate graph documents. The build executor
does not sleep, debounce, retry, or consume filesystem events.

### State model

```text
Starting
  ├─ watcher ready ─> Synchronizing
  └─ native start failure ─> Polling fallback ─> Synchronizing

Synchronizing
  ├─ success, no queued events ─> Idle
  ├─ success, queued events ─> Settling
  └─ failure ─> Backoff

Idle
  ├─ relevant event ─> Settling
  └─ reconciliation due ─> Building

Settling
  ├─ quiet window elapsed ─> Building
  └─ maximum window elapsed ─> Building

Building
  ├─ success, no queued events ─> Idle
  ├─ success, queued events ─> Settling
  └─ failure ─> Backoff

Backoff
  ├─ retry due ─> Building
  └─ new events ─> Backoff with merged pending paths
```

Only one build executor call may be active. The first event received while a
build is active marks the scheduler dirty; later events join the same pending
batch. Completion schedules at most one follow-up build.

## Adaptive timing

- The default quiet-window baseline is 150 milliseconds.
- The first relevant event starts both a quiet deadline and an absolute batch
  deadline.
- Each additional relevant event moves the quiet deadline to 150 milliseconds
  after that event.
- The absolute deadline is 750 milliseconds after the first event and never
  moves.
- A build starts when either deadline expires.
- Events received during a build form a new batch with their own deadlines once
  the active build completes.
- A reconciliation build is requested every five minutes while watch mode is
  otherwise idle.

`--debounce SECONDS` remains supported. It replaces the 150-millisecond
baseline, and the maximum batch window becomes five times the configured
baseline, capped at five seconds. This keeps the existing option meaningful
without adding a second timing flag.

The polling interval remains independent of debounce timing.

## Startup lifecycle

Watch startup follows this order:

1. Resolve and canonicalize the project root.
2. Load and validate `.compass/config.toml`.
3. Compile watch filters and report configuration errors before long-running
   work begins.
4. Start the native event source.
5. If native startup fails, start content-aware polling and emit one warning
   explaining the fallback.
6. Begin collecting events.
7. Run an initial cached synchronization.
8. If changes arrived during synchronization, schedule one adaptive follow-up.
9. Enter the normal idle watch loop.

Starting the event source before synchronization closes the gap in which edits
could otherwise be missed. If outputs are already current, the build pipeline
leaves them untouched and watch mode reports `up to date`.

## Event handling

Access-only events are ignored. Relevant create, modify, remove, rename, and
metadata events contribute their normalized project-relative paths to an
ordered set. Duplicate paths do not extend a pending batch unless they arrive
as a later event and therefore represent continued activity.

Rename events retain both paths when the backend supplies them. The existing
incremental detector remains responsible for interpreting additions,
deletions, and renames from the current filesystem and manifest state.

Generated output paths never reach the scheduler, preventing recursive rebuild
loops. Scope and ignore filtering also happen before timing state changes, so
noise outside the configured corpus cannot delay a useful build.

## Failure recovery

### Watcher startup

If the recommended native watcher cannot start or attach to a path, Compass
attempts a content-aware `PollWatcher`. A successful fallback keeps the process
running and emits one warning containing the native error and active polling
interval. If both backends fail, startup returns an error.

### Event errors

Recoverable event errors are reported and the watcher continues. Repeated
identical errors are counted and summarized rather than printed for every
occurrence. A disconnected event channel is treated as a backend failure:
Compass attempts polling fallback once before terminating.

### Build errors

A build failure does not discard the pending batch or stop watch mode. Retry
delays are 1, 2, 4, 8, 16, and then 30 seconds. Additional failures stay capped
at 30 seconds. New events merge into the retained batch without resetting the
retry counter.

A successful build clears the retry counter and duplicate-error summary.
Atomic publication and existing incomplete-build guards remain authoritative,
so failed attempts cannot expose partial graph artifacts.

### Reconciliation

The five-minute reconciliation invokes the same incremental build even if no
event is pending. It catches dropped native events and backend-specific event
ambiguities. An unchanged reconciliation does not rewrite public artifacts.

## Shutdown

Ctrl+C stops acceptance of new work and cancels pending timers. If no build is
active, Compass exits immediately. If a build is active, Compass reports that
the atomic build is finishing, allows it to complete, emits the final result,
and then exits.

Retries and periodic reconciliation do not start after shutdown is requested.

## User experience

### Interactive terminal

Startup reports:

- resolved project root;
- configured include and exclude rule counts;
- native or polling backend;
- adaptive quiet and maximum windows;
- five-minute reconciliation interval;
- output directory.

Runtime status uses concise state-oriented messages:

```text
[compass watch] Starting native watcher…
[compass watch] Synchronizing current graph…
[compass watch] Up to date. Watching for changes.
[compass watch] 3 paths changed; settling for 0.15s…
[compass watch] Building…
[compass watch] Rebuilt: 142 nodes, 318 edges in 0.42s
[compass watch] Changes arrived during build; one follow-up queued.
[compass watch] Build failed; retrying in 2s: <reason>
[compass watch] Native watcher unavailable; using polling every 0.5s: <reason>
```

The implementation may update an active TTY status line in place, but it must
not require animation or color for comprehension.

### Redirected output

When standard output is not a terminal, every status is written as a complete
stable line with a timestamp. No carriage-return rewriting or spinner output is
used. Repeated identical errors produce an initial line and later summary
counts.

### CLI compatibility

- Adaptive native watching is the default for `compass watch`.
- `--poll` continues to force content-aware polling.
- `--debounce SECONDS` configures the adaptive baseline.
- Existing scope and build flags retain their meanings.
- No new required configuration is introduced.
- Help and operations documentation explain startup synchronization, adaptive
  timing, automatic fallback, retry behavior, and the recovery role of manual
  `compass update`.

Graphify compatibility mode retains its fixed three-second debounce, existing
messages, existing initial behavior, and macOS polling selection.

## Public status model

`WatchStatus` should expose lifecycle events without embedding terminal
formatting:

- watcher starting and active backend;
- initial synchronization;
- settling with pending count and remaining quiet window;
- build started with reason (`initial`, `changes`, `retry`, or
  `reconciliation`);
- batch details;
- rebuilt or up-to-date result;
- follow-up queued;
- backend fallback;
- retry scheduled;
- summarized event error;
- finishing active build;
- stopped.

Frontends decide how these states are rendered. Tests can therefore validate
scheduler behavior without parsing terminal control sequences.

## Verification

Scheduler tests use a controllable clock and synthetic events rather than
wall-clock sleeps. They cover:

- one isolated event;
- burst coalescing;
- the 750-millisecond maximum delay;
- duplicate paths;
- events received during a build;
- rename and delete batches;
- reconciliation scheduling;
- bounded retry backoff;
- repeated-error summarization;
- shutdown while idle, settling, backing off, and building.

Filter tests verify saved scope, Git ignores, explicit exclusions, generated
outputs, sensitive files, and program artifacts before events reach the
scheduler.

Integration tests cover:

- initial synchronization with missing output;
- unchanged startup output;
- a change received during initial synchronization;
- automatic fallback when native watcher startup is unavailable;
- transient build failure followed by recovery;
- polling end-to-end behavior;
- clean interrupted shutdown.

Native backend tests validate startup and configuration but do not assert exact
OS event sequences. One polling end-to-end test remains the portable event
oracle.

Existing Graphify compatibility tests must remain byte-for-byte stable.

## Acceptance criteria

- An isolated relevant save begins building approximately 150 milliseconds
  after the last event.
- Continuous relevant edits trigger a build no later than approximately 750
  milliseconds after the first event in the batch.
- No more than one build runs concurrently.
- Events arriving during a build cause at most one immediate follow-up batch.
- Watch startup produces a current graph without requiring an edit.
- Native startup failure transparently falls back to polling when possible.
- A transient build failure recovers without restarting watch mode.
- Scope, ignore, safety, and generated-output filtering remain consistent with
  one-shot updates.
- Unchanged startup and reconciliation builds do not rewrite public artifacts.
- Ctrl+C never leaves a partially published artifact bundle.
- Graphify compatibility tests remain unchanged.

## Future extension

The build executor boundary may later accept a normalized change set and return
whether a full reconciliation is required. That permits file-level graph
patching or deferred clustering without changing the event source, scheduler,
status model, fallback behavior, or terminal UX defined here.
