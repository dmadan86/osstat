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
