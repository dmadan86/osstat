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

Model files follow the same rule, with one difference worth stating. They are
verified against a SHA256 pinned in this repository and a mismatch aborts with
no override, exactly as the runtime does. But the pins point at **community
GGUF re-uploads** — third parties who re-quantised a model — rather than at the
model vendors' own repositories, because the vendors' are gated behind an
account and a licence acceptance. The publisher of every pinned file is recorded
in the registry and named in the download control, so who you are trusting is
visible rather than implied. Models are stored wherever you chose in Settings,
and deleting one removes the file.

A way to make osstat execute a binary that does not match its pinned hash is a
critical vulnerability. So is a way to make an archive write outside its
directory, or to reach the runtime's HTTP port from off the machine.

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
- Dies when you leave the chat page, and is reaped on next launch if osstat was
  killed before it could. Reaping re-reads the recorded PID's start time and
  refuses on a mismatch, so a recycled PID is never signalled (threat 1's
  identity check, reused).

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
