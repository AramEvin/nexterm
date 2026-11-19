# Security policy

NexTerm stores credentials and connects to production servers, so security
reports get top priority.

## Supported versions

NexTerm is pre-1.0. Only the **latest release** receives security fixes.

| Version        | Supported |
| -------------- | --------- |
| latest 0.x     | ✅        |
| older releases | ❌        |

## Reporting a vulnerability

**Please do not open a public issue, discussion or pull request for security problems.**

Report privately through GitHub:
**[Security → Report a vulnerability](https://github.com/AramEvin/nexterm/security/advisories/new)**.

Please include:

- the affected version (Help → About, or the installer file name) and Windows version
- what an attacker can achieve, and the steps or a proof of concept
- whether it needs a malicious server, a local attacker, or user interaction

What to expect:

| Step                               | Target         |
| ---------------------------------- | -------------- |
| Acknowledgement                    | within 3 days  |
| First assessment (severity, scope) | within 7 days  |
| Fix released for high / critical   | within 30 days |

We'll keep you updated, credit you in the release notes and the advisory (unless you
prefer to stay anonymous), and coordinate disclosure with you.

## Scope

In scope: the NexTerm app and installer, and anything in this repository, for
example credential handling and the vault, host-key and certificate verification,
IPC command validation, file-transfer path handling, and the logging of secrets.

Out of scope: vulnerabilities in third-party servers you connect to, issues that
need an already-compromised Windows account with full control, and missing
hardening that has no demonstrated impact.

## Design

The security architecture and threat model are documented in
[docs/SECURITY.md](docs/SECURITY.md).
