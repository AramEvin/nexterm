# Test fixtures

`test_ed25519` / `test_ed25519_enc` are throwaway keys generated **for tests
only** (`ssh-keygen -t ed25519`; the second is the same key protected with the
passphrase `test-passphrase`). They grant access to nothing. Never use them for
real systems.
