# Download controls — implementation plan

**Goal:** A progress bar with rate and ETA, Pause/Resume, bounded automatic
retry, and Cancel, for model downloads.

**Approved design** (this file is the whole spec — the feature is an increment
on machinery that already exists):

- **Progress bar** reuses `ui/src/components/Meter.tsx`, the component the
  Overview draws CPU and RAM with. Below it, transfer rate and estimated time
  derived from the `model:progress` events already emitted.
- **Pause** stops the transfer and **keeps** the `.part`. **Cancel** stops and
  **deletes** it. That is the only difference between them. Resuming continues
  via HTTP `Range`, which `download_resumable` already does.
- **Retry:** at most three attempts with increasing delay (1s, 4s, 16s),
  resuming from the partial each time, then stop and show a Retry control.
- **Cell states:** not pinned → downloadable → downloading (bar, Pause, Cancel)
  → paused (bar, Resume, Cancel) → failed (Retry, Cancel) → downloaded (Run).

## Global Constraints

- **Only transient failures retry.** `AcquireError::ChecksumMismatch` and
  `AcquireError::HttpStatus` are permanent — a wrong pin or a file removed
  upstream. Retrying them wastes a minute and reports a permanent problem as a
  blip. Only network and I/O errors retry.
- **No `unwrap()`/`expect()`/`panic!()` outside `#[cfg(test)]`.** Tests opt out
  with exactly `#![allow(clippy::unwrap_used, clippy::expect_used)]`. **Never
  add `clippy::panic`** — the workspace sets it to `warn` and CI runs
  `-D warnings`, so `panic!` and `assert!(false, …)` fail even in tests.
- **No user data in logs** — log `error.kind()`, counts and durations only.
- **No new front-end dependency.** `ui/package.json` must be unchanged.
- **`git commit -s`**, conventional-commit prefix, no co-author trailers, Bash
  heredocs for multi-line commit messages.
- `just ci` must pass; `just bindings` must leave no diff.

---

## Task 1: Retry policy and pause in the Rust layer

**Files:** `crates/osstat-inference/src/download.rs` (retry helper),
`src-tauri/src/models.rs` (pause command, wiring).

**Produces:**

- `pub const fn is_transient(error: &AcquireError) -> bool`
- `models_pause` command; `models_cancel` keeps its delete-the-part behaviour.
- `model:progress` payload gains `bytes_per_second: Option<u64>` and
  `seconds_remaining: Option<u64>`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_wrong_hash_is_permanent_and_must_not_be_retried() {
        // THE load-bearing test. Retrying a wrong pin three times turns a
        // one-line "the pin is wrong" into a minute of apparent activity that
        // ends in the same place, and trains the user to distrust the message.
        let error = AcquireError::ChecksumMismatch {
            file: "model.gguf".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };
        assert!(!is_transient(&error));
    }

    #[test]
    fn a_missing_file_upstream_is_permanent() {
        assert!(!is_transient(&AcquireError::HttpStatus {
            url: "https://example.invalid/x".to_owned(),
            status: 404,
        }));
    }

    #[test]
    fn a_dropped_connection_is_transient() {
        // Build a Network error however AcquireError constructs one; a reqwest
        // error can be produced by requesting an unroutable address.
    }

    #[tokio::test]
    async fn a_permanent_failure_makes_exactly_one_request() {
        // Proves the policy end to end rather than in isolation: a fixture
        // serving a body that does not match the pin must be hit once, not
        // four times. Count requests in the fixture and assert 1.
    }

    #[tokio::test]
    async fn a_transient_failure_resumes_rather_than_restarting() {
        // The second request must carry a Range header, or "retry" is
        // "start again" and a 5 GB download restarts from zero.
    }
```

- [ ] **Step 2: Run and confirm they fail.**
- [ ] **Step 3: Implement.** `is_transient` matches on the variant — **no
      wildcard arm**, so a new error variant must be classified deliberately
      rather than defaulting to retryable. The retry loop lives in
      `src-tauri/src/models.rs` around the existing download call, with
      `tokio::time::sleep` between attempts.

      Pause and cancel share a cancellation token; they differ only in whether
          the `.part` is removed afterwards. Add a doc comment saying so, because
          the difference is one line and will otherwise look accidental.

          Rate and ETA are computed from consecutive progress events over a short
          window rather than the whole transfer, so a stall is visible instead of
          being averaged away.

- [ ] **Step 4: Run tests + `cargo clippy --workspace --all-targets --all-features -- -D warnings` + `cargo fmt --all --check` + `just bindings`.**

- [ ] **Step 5: Prove the permanent-failure test.** Make `is_transient` return
      `true` for everything, confirm
      `a_permanent_failure_makes_exactly_one_request` FAILS with a count of 4,
      paste the output, restore, confirm green.

- [ ] **Step 6: Commit** `feat(models): pause a download, and retry only what is worth retrying`

---

## Task 2: The controls in the advisor

**Files:** `ui/src/pages/Llm.tsx`, `ui/src/pages/Llm.download.test.tsx`.

Read `src-tauri/src/models.rs` and the regenerated bindings for real command and
field names first.

- [ ] **Step 1: Write the failing tests.** Assert on **content**:

```tsx
it('shows a progress bar with the fraction downloaded', async () => {});
it('shows transfer rate and estimated time while downloading', async () => {});
it('offers Pause while downloading and Resume once paused', async () => {});
it('keeps the progress figure across a pause, so nothing looks lost', async () => {});
it('says a permanent failure is a bad pin, not a network problem', async () => {
  // The user-visible half of the retry policy. "Download failed, retrying"
  // for a wrong hash would be a lie.
});
it('offers Retry after the attempts are exhausted', async () => {});
it('offers Run once downloaded', async () => {});
```

- [ ] **Step 2: Run and confirm they fail.**
- [ ] **Step 3: Implement** the six cell states using `Meter` for the bar.
- [ ] **Step 4: Tests, typecheck, lint.**
- [ ] **Step 5: Prove one test by deliberate breakage** — swap Pause and Resume
      labels, confirm the pause test fails, paste it, restore.
- [ ] **Step 6:** confirm `git diff --stat ui/package.json` is empty.
- [ ] **Step 7: Commit** `feat(ui): pause, resume, retry and cancel a download`
