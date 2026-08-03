# Model acquisition — design

**Status:** Approved, not yet implemented.
**Date:** 2026-08-03
**Covers:** ROADMAP M4.2, narrowed. See [Scope](#12-scope).

## 1. Context

ADR-008 built an advisor that says whether a model fits. ADR-013 made models
runnable, from a file the user already had. Nothing acquires one.

The gap is concrete, and it is data before it is code. A registry entry is:

```json
{
  "id": "llama-3.1-8b",
  "name": "Llama 3.1 8B",
  "family": "Llama",
  "parametersBillion": 8.03,
  "architecture": { "numLayers": 32, "hiddenSize": 4096, "...": "..." },
  "sourceNote": "Meta Llama 3.1 model card ..."
}
```

Fifteen models at four quantizations is sixty cells in the fit matrix, and not
one maps to a file anywhere: no repository, no filename, no hash, no size. The
registry was built to price models, not to fetch them.

### 1.2 Scope

The roadmap's M4.2 reads "Hugging Face search, resumable GGUF download, local
file import, GGUF header parsing reconciled against the registry." Local import
and header parsing landed with ADR-013. This spec covers **resumable download of
pinned files**, and deliberately **not** search.

Not in this spec, each a separate decision:

- Hugging Face search or repository browsing.
- Pasting an arbitrary GGUF URL.
- Hugging Face authentication tokens.
- Concurrent downloads.

## 2. Decisions

| Question              | Decision                                                             |
| --------------------- | -------------------------------------------------------------------- |
| Where files come from | Pinned per model+quant in the registry, like `runtimes.json`.        |
| Gated models          | Pin ungated community GGUF re-uploads; name the publisher in the UI. |
| Default location      | `<app-data>/models/`, beside `runtimes/`.                            |
| Configurable location | Folder picker in Settings; unset means the default.                  |
| Changing the location | Offer to move existing models, and move them if accepted.            |
| Verification          | SHA256 pinned in the repository. Mismatch aborts with no override.   |
| Running               | A downloaded cell hands its path to `chat_open_model` and navigates. |

### Why pinned rather than searched

ADR-012 argued this for the runtime and the argument transfers: a digest fetched
from the same origin as the file "proves the transfer was not corrupted, not
that the file is the one anybody reviewed." A hash in this repository means
compromising an upload cannot change what osstat will execute, and changing it
is a pull request whose diff a person can read.

It also matches what the roadmap already requires of the registry — "data, not
code", a file that "can ship without a release". Adding a model stays a data
change.

The cost is curation: sixty entries to fill, and pins that age as uploaders
re-quantise. That is the same trade ADR-012 accepted for `runtimes.json`, whose
tag "moved twice while this design was being written."

### Why ungated community re-uploads

Llama and Gemma are gated on Hugging Face: an account, a licence acceptance in a
browser, and a token. Pinning official repositories would mean nothing downloads
until all of that is done, and osstat would gain a stored credential — a new
secret to protect and a new threat entry.

Community GGUF publishers (`bartowski`, `lmstudio-community`, `ggml-org`) are
not gated, so downloads work with no account.

**This is a real trade and the UI must not hide it.** It means trusting a
third-party re-quantiser, and it steps around a gate the licensor put there
deliberately. The registry therefore records the publisher for every file, and
the download control names it. Presenting a community re-upload as though it
were the vendor's own would be the dishonest version of this feature.

## 3. Registry format

`models.json` gains a `downloads` array per model, one entry per available
quantization. A model with no entry for a quantization simply has no download
for that cell — the matrix still prices it.

```json
"downloads": [
  {
    "quantId": "Q4_K_M",
    "repo": "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
    "publisher": "bartowski",
    "file": "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
    "sha256": "…",
    "sizeBytes": 4920734368
  }
]
```

The URL is derived, not stored:
`https://huggingface.co/{repo}/resolve/main/{file}`. Storing a full URL would
let the host vary per entry, which is a wider trust surface than this feature
needs and one nobody would notice widening.

`models.schema.json` is extended and validated in the same test that already
validates the registry, and a drift test asserts every `quantId` names a
quantization the registry defines.

## 4. Storage and the model store

`ModelStore`, alongside the existing `RuntimeStore` in `osstat-inference`.

**A downloaded model is recorded by absolute path**, not by folder plus name.
That is what makes the move safe: an interrupted move leaves records pointing at
files that exist.

The index lives in `<app-data>/models.json` — deliberately not inside the model
folder, which the user may move, empty, or point at a network share.

Each record holds: model id, quantization id, absolute path, size, SHA256,
publisher, repository, and when it was acquired.

### Changing the folder

1. Settings offers a folder picker.
2. If models exist, osstat states the count and total size and asks before
   moving anything.
3. **Same volume:** `fs::rename` per file — instant and atomic.
4. **Different volume:** copy with progress, verify the SHA256 at the
   destination, **then** delete the source.
5. Cancellable at any point.

**The source is never deleted before the destination verifies.** A failure at
any point leaves the original in place and the library usable — the same
discipline `download_verified` already applies to a partial download.

## 5. Downloading

`osstat-inference` is extended rather than a sixth crate created. It already
owns verified acquisition: `download_verified` streams and hashes, moves into
place only on a match, and leaves nothing behind on failure; `fs4` checks free
space; `RuntimeStore` lists and deletes. A model is another artifact fetched and
hash-checked. A new crate would duplicate all of it to gain nothing.

### Sequence

1. **Free space is checked** against the pinned size before any bytes are
   fetched. `fs4` is already used this way before a 642 MB runtime; a 40 GB
   model deserves it more.
2. **Download to `<file>.part`**, resuming with an HTTP `Range` request from
   whatever is already there.
3. **Hash the completed file in one pass**, then move it into place.
4. **A mismatch deletes the partial** and reports. No retry, no fallback, no
   override.

### Why hashing moved to the end

The runtime downloader hashes as bytes stream past, which is free but cannot
resume: a dropped connection restarts from zero. That is tolerable for 34 MB and
not for 20 GB, which is an hours-long operation that _will_ be interrupted.

Resuming across sessions means the incremental hash state cannot be carried, so
the complete `.part` is hashed in one pass before being moved. The cost is one
extra full read — roughly half a minute for 20 GB — in exchange for surviving a
dropped connection. The trade is worth making at this size and would not be at
the runtime's, which is why the two differ.

### One at a time

Two 20 GB downloads competing for bandwidth and disk both finish later than one
after the other, and a progress bar that cannot say which file it describes is
worse than no progress bar. A second request while one is running is refused
with a message naming what is already downloading.

## 6. The advisor

Each cell gains a state: **not downloaded**, **downloading**, **downloaded**.
The verdict and the explanation drawer are untouched — this adds an action, it
does not replace the arithmetic.

**Download is offered even where the advisor says the model will not fit.** This
is the rule ADR-013 already follows: the calculator is explicitly an estimate,
and refusing on an estimate would make osstat wrong in a way the user cannot
override. The verdict is shown beside the control.

A cell with no pinned entry shows no control and says the file is not pinned,
rather than showing a control that fails.

The download control names the publisher, per §2.

Progress reaches the UI as `model:*` events, following `runtime:*` in
`src-tauri/src/runtime.rs` — same event-constant convention, same payload shape,
same spawned-task structure.

## 7. Running a downloaded model

A downloaded cell gets a **Run** control. It passes the record's absolute path
to the existing `chat_open_model` command and navigates to the chat page.

**There is one path into a session, not two.** The file picker and the advisor
both call the same command with a path, so everything ADR-013 established —
header parsing, the launch arithmetic, the lockdown argument vector,
supervision, teardown — applies unchanged. The chat page needs no modification.

**The header still wins over the registry.** The pin claims a cell is "Llama 3.1
8B at Q4_K_M"; the file's header says what it actually is, and `plan` computes
`-ngl` and `-c` from the header. Where the two disagree — a re-quantiser
uploaded something other than what the entry claimed — the header is used and
the disagreement is surfaced, because the header describes the bytes that will
be loaded.

## 8. Error taxonomy

Extends `AcquireError` in `osstat-inference` rather than introducing a parallel
enum.

| Case                              | Behaviour                                                             |
| --------------------------------- | --------------------------------------------------------------------- |
| No pinned entry for this cell     | No control; the cell says so.                                         |
| Not enough free space             | Refused before any bytes are fetched, naming the shortfall.           |
| Connection lost mid-download      | The `.part` survives; the next attempt resumes.                       |
| Server does not honour `Range`    | Restart from zero rather than corrupt the file.                       |
| SHA256 mismatch                   | Partial deleted, reported. No retry, no override.                     |
| A download is already running     | Refused, naming the file in progress.                                 |
| Move interrupted                  | Source intact, library usable, records unchanged.                     |
| Model file deleted outside osstat | The record is reported as missing, not silently re-listed as present. |

## 9. Testing

Hermetic. ADR-012: "CI does not download a runtime or run inference." A 20 GB
model makes that non-negotiable.

- **Schema and drift.** `models.json` validates against its schema, every
  `quantId` names a defined quantization, and every entry has a complete
  repo/file/sha256/size. A model deliberately without downloads is asserted as
  deliberate rather than missing.
- **Resume.** Against the existing local fixture server: sever the connection
  part-way, assert the retry carries the correct `Range` header, and assert the
  reassembled file hashes correctly. The fixture must serve the second request
  as a genuine partial-content response, or the test proves nothing.
- **A server ignoring `Range`** — replying `200` with the whole body to a ranged
  request — must restart cleanly rather than append to the existing `.part` and
  produce a corrupt file that fails its hash for a confusing reason.
- **Hash mismatch leaves nothing usable**, matching the existing runtime test.
- **Free space** is checked before any request is made, proven by asserting no
  request reached the fixture server.
- **The move, both ways.** A same-volume rename, and a cross-volume copy with a
  failure injected after the copy and before the delete — asserting the source
  survives and the library still works.
- **A missing file** is reported as missing rather than listed as present.

Real downloads are a manual gate. The platforms actually verified are named in
the report rather than implied by a green build.

## 10. Consequences

- **osstat downloads multi-gigabyte files it will later execute as model
  weights.** The pinned hash is the control, identical to the runtime's.
- **SECURITY.md gains a note** under the existing threat 5: models are verified
  against a hash in this repository, come from named third-party publishers, and
  are stored wherever the user chose.
- **The registry is now partly a distribution manifest**, and its pins age. The
  refresh procedure mirrors ADR-012's for `runtimes.json`.
- **The community-re-upload trade is visible in the UI**, not buried in a
  design document.
- **No new crate.** `osstat-inference` gains `models` and `ModelStore`.
- **An ADR is not warranted.** This applies ADR-012's decisions to a second
  artifact rather than making new ones; ADR-012 gains a short amendment
  recording that models follow the same rules, and why hashing moved to the end.

## 11. Known risks

- **Sixty pins to curate, and none can be verified without downloading.**
  Filling the registry means fetching each file's real SHA256 once. Until that
  happens the entries do not exist, and a wrong hash makes a model
  undownloadable in a way that looks like a network fault. The manifest should
  be populated incrementally, with the models most likely to be wanted first,
  rather than all sixty guessed at once.
- **Pins age faster than the runtime's.** Re-quantisers re-upload; a file can be
  replaced under the same name, which turns a correct pin into a mismatch. The
  error message must distinguish "the file changed upstream" from "your download
  was corrupted", or every stale pin will be reported as a bug.
- **The development machine has no model and limited disk.** Resume, move and
  free-space paths will be tested against fixtures and small files. A real
  20 GB download over a real interrupted connection will not have been exercised
  before this ships, and that should be reported rather than assumed.
