# Contributing to NexTerm

Thanks for helping! This guide covers how to report problems, propose changes and
get a pull request merged.

## Ways to help

- **Report a bug** with the [bug report form](https://github.com/AramEvin/nexterm/issues/new?template=bug_report.yml).
  Attach the log from `%APPDATA%\NexTerm\logs` (it contains no passwords or file names).
- **Suggest a feature** with the [feature request form](https://github.com/AramEvin/nexterm/issues/new?template=feature_request.yml).
  Check the [roadmap](docs/ROADMAP.md) first; it may already be planned.
- **Security issues:** never in public. Follow [SECURITY.md](SECURITY.md).
- **Code, tests, docs:** pick an issue labelled `good first issue` or `help wanted`, or
  open an issue first for anything larger than a small fix, so we can agree on the approach.

## Development setup

Windows is the main target; Linux works for development and tests.

1. Install the toolchain: [docs/SETUP-WINDOWS.md](docs/SETUP-WINDOWS.md)
   (Node.js LTS, Rust stable, MSVC build tools; on Linux the Tauri WebKitGTK packages).
2. `npm ci`, then `npm run tauri dev`.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before larger changes, and
[docs/SECURITY.md](docs/SECURITY.md) before touching credentials, IPC or networking.

## Rules every change must follow

These keep NexTerm safe. Pull requests that break them won't be merged.

- **Secrets stay in Rust.** Passwords and keys are resolved in Rust by credential id;
  they are never sent to the UI, logged, serialised, or written to disk unencrypted.
  Use `SecretString` / `Zeroizing`.
- **Validate every IPC command in Rust** (lengths, formats, paths), even when the UI already does.
- **Least privilege:** each new command is listed in `src-tauri/build.rs` and allowed
  explicitly in `src-tauri/capabilities/main.json`. No shell or opener plugins.
- **No `unsafe`**, and no `unwrap()` / `expect()` outside tests.
- **Logs** record connection events only: never secrets, terminal content or file names.
- **Untrusted input from servers** (file names, banners, listings) is validated before use.
- **Dependencies:** explain why a new crate or package is needed; prefer well-maintained,
  widely used ones. CI runs `cargo audit` and `npm audit`.

## Checks

Run these before pushing; CI runs the same ones:

```bash
npm run format:check && npm run lint && npm run typecheck && npm test && npm run build
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

### Integration tests

These connect to real servers. Locally:

```bash
# OpenSSH on 2222 (password + key) and 2223 (keyboard-interactive only); see .github/workflows/ci.yml
NEXTERM_TEST_SSH_HOST=127.0.0.1 NEXTERM_TEST_SSH_PORT=2222 \
NEXTERM_TEST_SSH_USER=ntest NEXTERM_TEST_SSH_PASSWORD=Test-Pass-123 \
  cargo test --test ssh_integration --test sftp_integration -- --ignored --test-threads=1

# FTP on 2121 and FTPS on 2990 (pip install pyftpdlib pyopenssl)
python3 scripts/test-ftp-server.py 2121 /tmp/ftp plain &
python3 scripts/test-ftp-server.py 2990 /tmp/ftps tls cert.pem key.pem &
NEXTERM_TEST_FTP_PORT=2121 NEXTERM_TEST_FTPS_PORT=2990 \
NEXTERM_TEST_FTP_USER=ftest NEXTERM_TEST_FTP_PASSWORD=Ftp-Pass-123 \
  cargo test --test ftp_integration -- --ignored --test-threads=1
```

The passwords above belong to throw-away test servers only.

## Pull requests

- One topic per pull request; keep it small enough to review.
- Use [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(sftp): …`, `fix(ui): …`, `docs: …`, `test: …`, `chore(deps): …`.
- Add or update tests: Rust unit tests next to the code, integration tests in
  `src-tauri/tests/`, Vitest tests in `src/**/__tests__/`.
- Update `CHANGELOG.md` (Unreleased section) and the docs if behaviour changes.
- UI changes: include a screenshot.
- By contributing, you agree that your contribution is licensed under the
  [Apache License 2.0](LICENSE).

## Releases (maintainers)

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`,
   then `npm install --package-lock-only` and `cargo update -p nexterm`.
2. Move the Unreleased changelog entries under the new version.
3. Tag and push: `git tag v0.7.0 && git push origin v0.7.0`. The release workflow builds
   the Windows installers on GitHub, attaches them with `SHA256SUMS.txt` and build
   provenance, and publishes the release notes from the changelog.
