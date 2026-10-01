# Security Policy

Waly's core promise is *provable* privacy: local-only execution and a
kernel-level network seal (`crates/waly-seal`). Security reports are
therefore taken especially seriously — a bypass of the seal, of the
no-persistence invariants (pixels, raw OCR, audio), or an escalation via the
SYSTEM service is a critical bug.

## Supported versions

Pre-alpha: only the `main` branch is supported.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's
**private vulnerability reporting** (repository Security tab → *Report a vulnerability*) with:

- a description and impact assessment,
- reproduction steps (machine, Windows version, Waly commit),
- if it concerns the seal: the relevant `audit_sceau` journal entries.

You should receive an acknowledgment within 7 days. Coordinated disclosure
is appreciated; credit will be given in the release notes unless you prefer
otherwise.

## Scope notes

- The WebView2 UI process is a shared OS component and sits **outside** the
  WFP seal by design; it is hardened via CSP and browser flags. Reports on
  that hardening are in scope.
- The honest guarantee/non-guarantee list lives in
  `docs/ONEPAGER-2026-07-21-huis-clos-conformite.md`.
