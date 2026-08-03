# Security Policy

osstat deletes files, kills processes and requests elevated privileges. That
makes its security properties more important than its features. This document
describes what we defend, how to report a problem, and what we consider a
vulnerability.

## Reporting a vulnerability

**Do not open a public issue.**

Report privately through GitHub's private vulnerability reporting:

> [Report a vulnerability](https://github.com/dmadan86/osstat/security/advisories/new)

Please include the affected version and platform, what an attacker gains, and
reproduction steps or a proof of concept.

**What to expect:** an acknowledgement within 5 working days, an assessment
within 10, and a fix released before public disclosure. We will credit you in the
advisory unless you prefer otherwise. There is no bounty programme.

## Supported versions

| Version | Supported |
| ------- | --------- |
| `main`  | Yes       |

Pre-1.0, only the latest release and `main` receive fixes. This table will be
replaced with a real support window at v1.0.

## Threat model

osstat runs as a desktop application under the user's own account. It assumes the
operating system, the kernel and the user's account are not already compromised —
if they are, nothing osstat does matters.

What we actively defend against:

### 1. Privilege escalation through the elevation helper

The app **never runs elevated**. Operations needing admin or root invoke a
separate helper for that one operation (ADR-006). The helper accepts a **narrow,
versioned command protocol** — for example "kill this PID", "delete the files in
this validated manifest" — and never arbitrary paths or shell commands.

This narrowness _is_ the security control. Any change that widens the protocol
requires an ADR update and explicit review. Treat a way to make the helper run
attacker-chosen code or delete attacker-chosen paths as a critical vulnerability.

### 2. Destructive path traversal in cleaning rules

Cleaning rules are data (TOML manifests), which means a malicious or simply wrong
rule is a threat. The engine enforces, and no manifest can override:

- A hard denylist of system roots (`C:\Windows`, `/System`, `/usr`, `/etc`,
  `/boot`) and user document folders.
- Path canonicalization before deletion, so a symlink cannot redirect a delete
  outside its pattern root.
- Refusal to act outside the expanded roots of the rule's own patterns.

A rule or crafted filesystem layout that causes a delete outside these bounds is
a critical vulnerability. So is any way to make the scanner follow a symlink out
of its root.

### 3. Webview compromise reaching the OS

The front-end has no direct OS access. It can only call commands explicitly
allow-listed in `src-tauri/capabilities/` (ADR-001), and a strict Content
Security Policy is set in `tauri.conf.json`. Loading remote content into the
webview, weakening the CSP, or exposing a command that takes an unvalidated path
are all security-relevant changes.

### 4. Supply chain

`cargo audit`, `cargo deny` and `npm audit` run on every pull request and on a
weekly schedule. Dependencies are reviewed on addition, and release artifacts are
published with SHA256 sums.

### 5. Executing a downloaded inference runtime

The LLM features need a llama.cpp server binary. It is too large to bundle
within the installer budget, so it is fetched when a user asks for it
(ADR-012). osstat will only execute a build whose SHA256 matches a hash
**pinned in this repository**, in `crates/osstat-inference/runtimes.json`.

The hash is compiled into osstat rather than fetched alongside the artifact.
A digest served from the same origin as the binary proves the transfer was not
corrupted; it does not prove the file is the one anybody reviewed.

The controls:

- A mismatched archive is deleted and never extracted, never marked executable
  and never run. There is no retry and no override — retrying a hash that did
  not match is not a recovery strategy.
- Archive entries that would be written outside the runtime directory are
  refused, exactly as cleaning-rule paths are (threat 2).
- The runtime runs as the user, never elevated, and never through a shell.
  Arguments are passed as a vector, so a model path cannot become a command.
- It binds `127.0.0.1` on an ephemeral port, never `0.0.0.0`.
- Downloads land in a temporary file and are moved into place only after the
  hash matches, so nothing partial can be mistaken for a usable runtime.
- What was downloaded is listed with its size in Settings and can be deleted.

Model files have **two verification tiers**, and osstat labels which one a model
came from rather than presenting both as equally checked.

**Pinned models** — the curated set in the fit matrix — are verified against a
SHA256 pinned in this repository, and a mismatch aborts with no override,
exactly as the runtime does. The pins point at **community GGUF re-uploads** —
third parties who re-quantised a model — rather than at the model vendors' own
repositories, because the vendors' are gated behind an account and a licence
acceptance. The publisher of every pinned file is recorded in the registry and
named in the download control, so who you are trusting is visible rather than
implied.

**Searched models** — anything you find through the search box — are verified
against the SHA256 Hugging Face reports beside the file. That detects a
corrupted transfer. It does **not** detect a compromised upload, because the
digest and the file come from the same origin: whoever could replace one could
replace the other. Nobody reviewed that hash in a pull request here.

That is a real difference and osstat says so plainly. Every searched result and
every model downloaded from one carries the label _Not reviewed · hash from
Hugging Face_, in the search results and in the model list afterwards. A
searched model is a choice you make knowingly, about a file you went looking for
and picked; a searched model that downloaded and displayed exactly like a pinned
one would retire this guarantee without telling anyone.

What is the same across both tiers: the hash is always checked, a mismatch
always aborts with no override and no retry, downloads land in a temporary file
and move into place only after the hash matches, and search results with no
usable hash are never offered at all. What differs is only where the hash came
from. Gated repositories stay unavailable — osstat sends no authentication
token — and split multi-part GGUF files are excluded rather than half-supported.

Models are stored wherever you chose in Settings, and deleting one removes the
file. **The webview makes no HTTP request for any of this**, including search:
it hands a term to Rust and gets results back.

A way to make osstat execute a binary that does not match its pinned hash is a
critical vulnerability. So is a way to make an archive write outside its
directory, to reach the runtime's HTTP port from off the machine, or to make a
searched model download without its label or with a hash osstat never checked.

### 6. Running a model, and what the chat keeps

Threat 5 covers acquiring the runtime. Running it adds two surfaces: a live
local HTTP server, and conversations stored on disk (ADR-013).

The server osstat starts:

- Binds `127.0.0.1` on an OS-allocated port, never a fixed one and never
  `0.0.0.0`.
- Carries a random per-session key, seeded from the OS. Nothing else on the
  machine can drive the model through that port without it. The key is never
  written to disk and never sent to the webview.
- Runs with `--no-webui`. `llama-server` serves its own chat application by
  default; osstat does not put an unrequested web app on a local port.
- Is **never** given `--tools` or `--agent`, which enable built-in tools
  including `exec_shell_command` and `write_file`.
- **Keeps running until you unload the model or quit osstat.** It used to stop
  when you left the chat page; that made the rest of the application unusable
  while a model was loaded, and ADR-013 records the reversal. Quitting ends it
  from `RunEvent::Exit`, and it is reaped on next launch if osstat was killed
  before it could. Reaping re-reads the recorded PID's start time and refuses on
  a mismatch, so a recycled PID is never signalled (threat 1's identity check,
  reused).
- **Holds its weights the whole time it is loaded**, which is several gigabytes
  for a 7B model and more for a larger one. osstat's stated budget of idle RAM
  under 150 MB describes osstat, not a model it is running for you; while one is
  loaded the machine is holding both. The navigation marks the Chat tab for as
  long as that is true, so a loaded model is never something osstat is holding
  silently — but the memory is genuinely held, and Unload is how you get it
  back without quitting.

What the chat stores:

- **Conversations are written to disk**, one JSON file each, under osstat's
  app-data directory (`%APPDATA%\dev.osstat.app\conversations` on Windows,
  `~/.local/share/dev.osstat.app/conversations` on Linux,
  `~/Library/Application Support/dev.osstat.app/conversations` on macOS).
- **They are not encrypted.** Anything you type into the chat, and anything the
  model replies, is readable by any process running as you.
- **Deleting a conversation deletes its file.** There is no tombstone and no
  recycle bin inside osstat.
- **Uninstalling does not remove them.** The app-data directory outlives the
  application, as it does for every other setting; delete it by hand if you want
  the conversations gone.
- Conversation identifiers arrive from the front end and are validated before
  they reach a path, so a crafted identifier cannot read or delete a file
  elsewhere on disk — the same rule threat 2 applies to cleaning paths.

A way to reach the inference port from off the machine is a critical
vulnerability, as is a way to make a conversation identifier escape its
directory. Conversations being readable by the user who wrote them is not a
vulnerability; it is what "stored on your computer, unencrypted" means, and it
is documented here so the choice is visible rather than discovered.

## Not vulnerabilities

- **Deleting files you told it to delete.** Preview-and-confirm is the control;
  confirming a deletion is a decision, not a bug.
- **Requiring admin rights** for operations that genuinely require them.
- **Reading system information** the current user can already read.
- Anything requiring an already-compromised machine or physical access.

## Privacy

osstat contains no telemetry, analytics or crash reporting, and makes no network
requests except ones you explicitly trigger. If you find network traffic that
contradicts this, report it as a vulnerability — because it would be one.

At present exactly one action reaches the network: downloading an inference
runtime from Settings, which happens only when you press the button (ADR-012).
There is no background polling, no prefetch, and no check for a newer upstream
build. All of it happens in Rust — the webview's Content Security Policy allows
it no network access at all, and this feature did not change that.

Chatting with a model makes HTTP requests to `127.0.0.1`, and those are not an
exception to the claim above. They reach a process osstat itself started, on a
loopback port; nothing leaves the machine, and no prompt or reply is sent
anywhere. Those requests are made from Rust too, so the webview still has no
network access of any kind (ADR-013).

## Logs

osstat writes a log file. **It contains no personal data at any level, and there
is no setting that enables any**, so it is safe to attach to a bug report
without reading it first.

- One file per day, `osstat.YYYY-MM-DD.log`, in a `logs` folder under the same
  app-data directory the conversations use (`%APPDATA%\dev.osstat.app\logs` on
  Windows, `~/.local/share/dev.osstat.app/logs` on Linux,
  `~/Library/Application Support/dev.osstat.app/logs` on macOS).
- **A week is kept**, oldest deleted first. Deleting the folder is safe at any
  time; it is recreated on the next launch.
- **Settings → Logs** chooses how much detail is captured and copies the whole
  set into a folder you name.

This is stricter than a redaction policy, and deliberately so, because of what
this application can see. A log that recorded "all events" here would contain
your process names and PIDs, your open ports with their remote addresses, the
paths you were cleaning, the models you run, and — at a debug level — your
prompts and the model's replies. The purpose of saving a log is to send it to
someone, and a design where the useful log is also the sensitive one puts you in
the position of choosing between getting help and disclosing all of that.

It is a property of the code rather than a promise anyone has to keep:

- **All logging goes through one module.** Every event function in
  `src-tauri/src/log.rs` takes counts, durations, booleans and fixed
  `&'static str` kinds. There is no `String`, `Path` or `PathBuf` parameter
  anywhere in its surface, so a path cannot be logged because there is nowhere
  to put one.
- **Errors log their variant, not their message.** Every error type has a
  `kind()` returning a fixed name — `spawn_failed`, `checksum_mismatch`,
  `not_a_gguf` — and only that is written. The messages themselves name files,
  paths and URLs, which is what makes them useful on screen and unacceptable in
  a log.
- **The filter is scoped to osstat's own code.** A bare level would also enable
  `reqwest`'s and `hyper`'s instrumentation, which log URLs — user data arriving
  through the back door at exactly the setting somebody turned on to help.
- **A test scans the output.** The suite captures every line the logging paths
  produce and fails on a path separator, an `@`, or a long run of digits.

The last of those is a net under the module boundary, not a guarantee, and
should not be read as more: it catches an accidental `{:?}` of a struct that
grew a path field later, which is how this would realistically break. It does
not catch a bare process name.

The cost is real and worth stating: a failed download logs `download_failed
kind=checksum_mismatch` with no file name, and if two downloads ran that session
the log does not say which. Some bugs need a reproduction rather than a log.
That is the accepted trade.

A log line that contains a path, a file name, a process name, an address, a
prompt or a model reply is a vulnerability. Report it.
