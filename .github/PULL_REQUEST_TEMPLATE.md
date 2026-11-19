## What and why

<!-- What does this change, and which issue does it close? e.g. "Closes #12" -->

## How it was tested

<!-- Unit / integration / manual steps. Screenshots for UI changes. -->

## Checklist

- [ ] Conventional Commit title (`feat(scope): …`, `fix(scope): …`)
- [ ] `npm run format:check && npm run lint && npm run typecheck && npm test` pass
- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass
- [ ] Tests added or updated
- [ ] `CHANGELOG.md` (Unreleased) and docs updated if behaviour changed

### Security (if relevant)

- [ ] No secret reaches the UI, logs or disk unencrypted
- [ ] New IPC commands validate input in Rust and are allowed in `capabilities/main.json`
- [ ] Data from servers (names, listings, banners) is validated before use
