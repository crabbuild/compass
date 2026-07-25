# Adaptive Compass Watch Implementation Plan

**Goal:** Make native `compass watch` feel immediate and recover gracefully while preserving the existing cached build as the correctness oracle.

**Architecture:** Add a small timing/state unit in `compass-core`, then use it from the existing watcher loop to coalesce events, run an initial synchronization, serialize builds, retry transient failures, and reconcile periodically. Keep Graphify compatibility on its current legacy path and render the richer native statuses in `compass-cli`.

**Tech stack:** Rust, `notify`, `std::sync::mpsc`, existing Compass build/cache pipeline.

## Global constraints

- Do not use TDD; add focused regression tests after each implementation unit.
- Do not introduce async runtimes, daemon processes, or new dependencies.
- Run at most one graph build at a time.
- Keep native Compass defaults at a 150 ms quiet window and a 750 ms maximum batch window.
- Preserve Graphify compatibility messages, three-second timing, initial behavior, and macOS polling choice.
- Preserve scope, ignore, safety, completeness, and atomic-publication behavior.
- Retain `--poll` and `--debounce SECONDS`.
- Keep unrelated generated and untracked files out of commits.

## File map

- Create `crates/compass-core/src/watch_scheduler.rs`: pure adaptive deadlines, retry backoff, reconciliation timing, and scheduler state.
- Modify `crates/compass-core/src/lib.rs`: register the scheduler module.
- Modify `crates/compass-core/src/watch.rs`: backend startup/fallback, initial sync, adaptive event loop, retry/reconciliation, richer statuses.
- Modify `crates/compass-cli/src/lib.rs`: native defaults, Graphify legacy options, status rendering, parser tests.
- Modify `crates/compass-cli/src/help.rs`: explain adaptive behavior and fallback.
- Modify `crates/compass-core` watcher tests: lifecycle and end-to-end recovery coverage.
- Modify `crates/compass-cli/tests/watch_cli.rs`: native startup synchronization and output coverage.
- Modify `docs/reference/commands.md` and `docs/guides/operations.md`: user-facing behavior.

## Task 1: Adaptive scheduler

- [ ] Add `WatchScheduler` with pending-path storage, first/last event timestamps, quiet/max deadlines, retry count, and next reconciliation deadline.
- [ ] Expose operations to record paths, decide the next wake duration, take a ready batch, retain work after failure, reset after success, and request reconciliation.
- [ ] Use retry delays of 1, 2, 4, 8, 16, and 30 seconds, capped at 30 seconds.
- [ ] Make `--debounce` derive a maximum window of five times the baseline, capped at five seconds.
- [ ] Add post-implementation unit tests using explicit `Instant` values for burst coalescing, maximum delay, retry, success reset, and reconciliation.
- [ ] Run `cargo test -p compass-core watch_scheduler`.

## Task 2: Resilient watch lifecycle

- [ ] Extend `WatchOptions` with native/legacy lifecycle selection and reconciliation timing without changing public build behavior.
- [ ] Start the event backend before native initial synchronization.
- [ ] Attempt `RecommendedWatcher` first and fall back to content-aware `PollWatcher` when native startup or root attachment fails.
- [ ] Attempt polling fallback once if the native event channel disconnects.
- [ ] Preserve explicit `--poll` behavior without emitting a fallback warning.
- [ ] Feed filtered paths into `WatchScheduler`; ignore access-only events and generated/noise paths before changing deadlines.
- [ ] Run native builds for initial synchronization, ready change batches, retries, and reconciliation.
- [ ] Keep Graphify compatibility on the previous fixed-debounce, no-initial-sync loop.
- [ ] Retain failed pending work and continue watching; emit one retry status with the bounded delay.
- [ ] Collapse repeated identical backend/build errors into a count and clear the summary after recovery.
- [ ] Stop scheduling new work after cancellation and emit a clean stopped status.
- [ ] Add post-implementation tests for initial sync, changes during/after sync, transient build failure state, and polling behavior.
- [ ] Run `cargo test -p compass-core watch`.

## Task 3: Native watcher UX

- [ ] Add typed `WatchStatus` variants for backend activation/fallback, synchronization, settling, build reason, up-to-date result, follow-up, retry, and stopping.
- [ ] Keep existing variants used by Graphify compatibility unchanged.
- [ ] Render concise native messages with resolved root, scope counts, backend, timing, output directory, build progress, fallback, and retries.
- [ ] Keep redirected output as complete lines; do not add animation, color, or terminal dependencies.
- [ ] Prefix redirected native status lines with stable timestamps while preserving the legacy Graphify byte contract.
- [ ] Set native defaults to 150 ms while explicitly assigning three seconds for Graphify compatibility.
- [ ] Update help copy to describe initial synchronization, adaptive maximum delay, automatic polling fallback, and `--debounce`.
- [ ] Extend parser/status tests after implementation.
- [ ] Run `cargo test -p compass-cli --test watch_cli --test help_cli` and the focused CLI unit tests.

## Task 4: Documentation and regression verification

- [ ] Update the command reference and operations guide with the new lifecycle and recovery behavior.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p compass-files`.
- [ ] Run `cargo test -p compass-core`.
- [ ] Run `cargo test -p compass-cli --test watch_cli --test help_cli --test update_cli --test init_cli --test compass_product`.
- [ ] Run `cargo clippy -p compass-core -p compass-cli --all-targets --no-deps -- -D warnings`.
- [ ] Run shell completion syntax checks and `git diff --check`.
- [ ] Run `graphify update .` from the parent repository.
- [ ] Review the final diff for Graphify output regressions and unrelated files.
- [ ] Commit the implementation with a focused message.
