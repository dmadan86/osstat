# Model search — design and implementation plan

**Covers:** searching public models on Hugging Face from the LLM tab and
downloading them, alongside the seven pinned entries.

## 1. The decision this rests on

Approved: **search is allowed, downloads are verified against Hugging Face's
own reported SHA256, and searched models are labelled as unreviewed provenance.**

Two tiers now exist and must stay visibly different:

|                          | Pinned (7 models)                           | Searched                                  |
| ------------------------ | ------------------------------------------- | ----------------------------------------- |
| Hash source              | This repository, reviewed in a pull request | Hugging Face API, same origin as the file |
| What verification proves | The bytes are the ones somebody reviewed    | The transfer was not corrupted            |
| Provenance               | Named publisher, curated                    | Whatever the search returned              |

ADR-012's reasoning stands and is not being overturned — it is being _scoped_.
A digest fetched alongside its file cannot prove the file is the one anybody
reviewed. For the runtime binary, which osstat **executes**, that was
unacceptable. For model weights the user went looking for and chose, it is a
trade the user makes knowingly, provided osstat says so plainly rather than
presenting both tiers as equally checked.

**The label is the feature.** A search result that downloads exactly like a
pinned one, with no visible difference, would quietly retire a guarantee
SECURITY.md still makes.

## 2. What the advisor can say about a searched model

> **Correction.** This section previously stated: _"The header is only available
> after downloading. So a searched result shows its file size and nothing
> more."_ **The first sentence was false, and the second followed from it.** A
> GGUF header sits at the **start** of the file, and HTTP has had a way to ask
> for the start of a file since 1999. osstat already had both halves of what
> that needs: `download_resumable` issues `Range` requests, and
> `osstat_chat::gguf::parse_prefix` reads a header out of a prefix while saying
> whether more bytes would help. Nothing had to be invented; the two pieces had
> simply not been put together.
>
> The original reasoning conflated "osstat has not read the header" with "the
> header cannot be read". Only the second would have justified withholding a
> verdict, and it was never true.

The fit matrix prices a model from its architecture — layer count, head counts,
context length — which lives in the registry for the seven pinned models and in
the **GGUF header** for everything else. That header is fetched with a `Range`
request against the candidate file, so a searched result is priced by the same
`plan_launch` a downloaded model is priced by, over the same bytes.

**There is one pricing path.** A searched result and the same file once
downloaded produce identical verdicts, because the only difference between them
is where the header bytes came from and `plan_launch` cannot tell.

### Why a range-fetched header, and not Hugging Face's `gguf` metadata

The API does expose a `gguf` field carrying `architecture`, `context_length` and
a `total` parameter count. It is not enough and must not be relied on: it
reports **no layer count, no head counts and no head dimension**, which are
three of the five terms the KV-cache and `-ngl` arithmetic needs. A verdict
assembled from it would be part measurement and part guess, with nothing in the
answer saying which part was which — a worse failure than showing no verdict,
because it would look exactly like the calculator's output.

Reading the header instead reads the actual bytes of the actual file, works for
any host rather than one API, and matches how osstat treats every other
measurement.

### What the ceiling is for

The read starts at 4 MiB and grows only while `parse_prefix` reports
`NeedMoreBytes`, stopping at the same **64 MiB** ceiling `src-tauri/src/chat.rs`
uses for a local file. A server that ignores `Range` and answers `200` with the
whole body is detected from the status, read to the ceiling and no further, and
the connection dropped mid-body — the result is then reported as **unpriced**.
That is the honest answer for a file whose header did not arrive, and it costs
64 MiB rather than thirty gigabytes.

### What is still forbidden

Deriving a parameter count from file size and quantization bits. That would
produce a number that looks like the calculator's and is a guess — precisely the
failure ADR-008 names as the worst thing this feature could do. **The rule was
never "show no verdict"; it was "never present a guess as a measurement."** The
range-fetch satisfies it by measuring. A row whose header could not be read
shows its size alone and says why, rather than filling the gap with arithmetic
over a quantization tag.

Because each read is a network request against a multi-gigabyte file, it is made
**per result and on demand** — when a row is expanded — and never eagerly for a
page of results.

## 3. Scope

**In:** a search box in the LLM tab, results from the Hugging Face API filtered
to GGUF files, download with HF-hash verification, and the provenance label
carried through the model list and Settings.

**Out:** authentication tokens (gated repos stay unavailable), multi-part GGUF
files, non-Hugging-Face sources, and any change to how the seven pinned models
behave.

## Global Constraints

- **No `unwrap()`/`expect()`/`panic!()` outside `#[cfg(test)]`.** Tests opt out
  with exactly `#![allow(clippy::unwrap_used, clippy::expect_used)]`. **Never
  add `clippy::panic`** — the workspace sets it to `warn` and CI runs
  `-D warnings`, so `panic!` and `assert!(false, …)` fail even in tests.
- **All network egress stays in Rust.** The webview issues no HTTP request —
  ADR-012's stated property, still true after ADR-013.
- **Logging carries no user data**: `error.kind()`, counts and durations only.
  Never a search term, repository name, or file name.
- **No new front-end dependency.** `ui/package.json` unchanged.
- **CI never reaches the network.** Every test uses the local fixture server.
- `git commit -s`, conventional-commit prefix, no co-author trailers, Bash
  heredocs for multi-line commit messages.
- `just ci` must pass; `just bindings` must leave no diff.

---

## Task 1: Searching Hugging Face

**Files:** create `crates/osstat-inference/src/search.rs`; modify `lib.rs`.

**Produces:**

- `pub struct SearchResult { pub repo: String, pub publisher: String, pub file: String, pub size_bytes: u64, pub sha256: String, pub quant_hint: Option<String> }`
- `pub async fn search(client: &reqwest::Client, query: &str, limit: usize) -> Result<Vec<SearchResult>, AcquireError>`

The API: `GET https://huggingface.co/api/models?search=<query>&filter=gguf&limit=<n>`
lists repositories; `GET https://huggingface.co/api/models/<repo>/tree/main`
lists files, where each entry carries `path`, `size`, and `lfs.oid` — **the LFS
oid is the file's SHA256**. This is the same call the seven pins were curated
with, so it is known to work.

- [ ] **Step 1: Write the failing tests** against the existing hand-rolled
      `TcpListener` fixture in `download.rs` — do not add a test dependency.

```rust
    #[tokio::test]
    async fn results_carry_a_hash_and_a_size() {
        // Without both, a result cannot be downloaded or verified, and
        // offering it would be offering a button that fails.
    }

    #[tokio::test]
    async fn a_file_without_an_lfs_oid_is_skipped_not_guessed() {
        // Small files are stored directly rather than in LFS and have no oid.
        // A result with no hash must be dropped, never downloaded unverified.
    }

    #[tokio::test]
    async fn only_gguf_files_are_returned() {
        // A repository holds READMEs, configs and safetensors. Offering a
        // .safetensors file would produce a download that cannot be loaded.
    }

    #[tokio::test]
    async fn a_multi_part_file_is_skipped() {
        // Names like `-00001-of-00002.gguf` are one shard of a split model.
        // Downloading one shard yields a file that fails its own hash check
        // for a reason nobody could diagnose. Out of scope, so exclude it.
    }

    #[tokio::test]
    async fn a_malformed_response_yields_no_results_rather_than_an_error() {
        // Search is a convenience; a bad response should show "nothing found",
        // not break the page.
    }
```

- [ ] **Step 2: Run and confirm they fail.**
- [ ] **Step 3: Implement.** `quant_hint` is parsed from the file name where it
      contains a known quantization tag, and is `None` otherwise — it is a
      display hint, never an input to any calculation.
- [ ] **Step 4: Gate.** `cargo test`, clippy `-D warnings`, `cargo fmt --check`.
- [ ] **Step 5: Prove the skip rule.** Make the multi-part filter accept
      everything, confirm `a_multi_part_file_is_skipped` FAILS, paste the
      output, restore.
- [ ] **Step 6: Commit** `feat(models): search Hugging Face for GGUF files`

---

## Task 2: Downloading a searched model

**Files:** `crates/osstat-inference/src/model_store.rs`, `src-tauri/src/models.rs`.

**Produces:** `ModelRecord` gains `pub provenance: Provenance` where
`pub enum Provenance { Pinned, Searched }`, and a `models_download_searched`
command taking a `SearchResult`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_record_written_before_provenance_existed_still_loads() {
        // Users have models on disk from before this feature. Losing them
        // would be worse than the feature is worth. `#[serde(default)]` with
        // Pinned as the default is correct: everything that exists today came
        // from the pinned registry.
    }

    #[test]
    fn a_searched_model_is_recorded_as_searched() {}

    #[test]
    fn a_searched_download_still_refuses_a_hash_mismatch() {
        // The weaker guarantee is about WHERE the hash came from, not about
        // whether it is checked. A corrupted transfer must still be rejected.
    }
```

- [ ] **Step 2: fail → Step 3: implement → Step 4: gate.** Reuse
      `download_resumable` unchanged: pause, resume, retry and the free-space
      check all apply to searched models exactly as they do to pinned ones.
- [ ] **Step 5:** `just bindings`, commit regenerated bindings.
- [ ] **Step 6: Commit** `feat(models): download a searched model, recording its provenance`

---

## Task 3: The search UI

**Files:** `ui/src/pages/Llm.tsx`, `ui/src/pages/Llm.search.test.tsx`.

Read the real command and payload names from `src-tauri/src/models.rs` and the
regenerated bindings first.

- [ ] **Step 1: Write the failing tests.** Assert on content:

```tsx
it('shows a searched result with its size and publisher', async () => {});
it('shows no verdict for a searched result until its header is asked for', async () => {
  // Replaces a test that asserted a searched result must NEVER show one —
  // see the correction in §2. The line that actually matters is narrower and
  // still holds: no verdict for a header that was not read. An unexpanded row
  // has not cost a request, so it has nothing to say beyond its size.
});
it('prices a searched result from its header when the row is expanded', async () => {
  // The same `plan_launch` a downloaded model is opened with, over a header
  // range-fetched from the front of the actual file.
});
it('falls back to the size alone when the header cannot be read', async () => {
  // A server that ignores `Range` produces this. Guessing a verdict from the
  // size to fill the gap is the one thing ADR-008 forbids.
});
it('reads a header once however often a row is opened and shut', async () => {});
it('marks a searched model as unreviewed, distinctly from a pinned one', async () => {
  // The label IS the feature. Without it this quietly retires a guarantee
  // SECURITY.md still makes.
});
it('says when a search returns nothing, rather than showing an empty area', async () => {});
it('still shows the seven pinned models with their verdicts', async () => {
  // Search must not displace the curated matrix.
});
```

- [ ] **Step 2: fail → Step 3: implement.** A search box above the fit matrix;
      results in their own section, visibly separate from the pinned matrix.
      Downloaded searched models appear in the same model list with their label.
- [ ] **Step 4: Tests, typecheck, lint.**
- [ ] **Step 5: Prove the label test** by removing the label, confirm it fails,
      paste, restore.
- [ ] **Step 6:** confirm `git diff --stat ui/package.json` is empty.
- [ ] **Step 7: Commit** `feat(ui): search public models from the LLM tab`

---

## Task 4: Documentation

- [ ] **SECURITY.md**, threat 5: state the two tiers explicitly — pinned models
      verified against a hash reviewed in this repository, searched models
      verified against a hash from the same origin as the file, which detects a
      corrupted transfer but not a compromised upload. Say that osstat labels
      which is which, and that a searched model is a choice the user makes.
- [ ] **ADR-012 amendment**: search is scoped, not an overturning — the pinned
      rule still governs the runtime binary and the curated models, and the
      reasoning for treating user-chosen weights differently.
- [ ] **ROADMAP.md**: M4.2 now includes search; note tokens and multi-part files
      remain out.
- [ ] `just ci`. If `lint-ui` fails only on paths under `.agents/`, that is
      pre-existing untracked debris — verify every path starts with `.agents/`
      and say so rather than "fixing" it.
- [ ] **Commit** `docs(models): record the two verification tiers`
