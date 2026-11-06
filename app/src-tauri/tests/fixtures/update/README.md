# TEST-ONLY update fixtures

`test.key` and `other.key` are throwaway minisign keys made for the auto-update tests
(`tauri signer generate --ci`, no password). They sign nothing real: the release key is the
`UPDATE_SIGNING_KEY` Actions secret and is never in the repo.

- `payload.bin` is a stand-in installer.
- `payload.bin.sig` signs it for version 0.1.1; `payload.v010.sig` for 0.1.0; `payload.nover.sig` carries no version.
- `payload.dev.sig` signs it for the dev build `0.1.2-dev.3` (the experimental channel's test build).
- `other.key.pub` is the "wrong key" for the tests.
