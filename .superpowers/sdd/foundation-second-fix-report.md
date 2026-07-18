# Foundation second fix report

## Finding disposition

- Important 1 — replaced the TCP-peer keyed limiter with a process-wide 4,096-attempt/15-minute sliding fuse and a four-permit `try_acquire_owned` authentication semaphore. Invalid request shapes are rejected before the fuse; peer and forwarded metadata are ignored; the existing bounded account delay remains soft-only and clears only after session creation.
- Important 2 — added `durable_replace(temp, config, data_dir)`. Unix syncs the temporary file, renames it, then syncs the parent directory. Windows uses `MoveFileExW` with replace-existing and write-through flags through target-specific `windows-sys 0.61.2`; other targets return `Unsupported` before database attachment or administrator commit. CI now compiles the Windows target.
- Minor 1 — configured startup and FULL setup classify user count together with the singleton bootstrap claim. Only zero/zero enters `ADMIN_ONLY` and users/claim enters `READY`; mismatches are explicit inconsistent-bootstrap errors. Deleting the claimed administrator no longer reopens setup.
- Minor 2 — administrator email accepts ASCII only, lowercases first, and validates the final persisted byte lengths. Unicode case expansion is rejected with the existing redacted field error.
- Minor 3 — an armed temporary-config RAII guard is created immediately after the secret file. Write, sync, permission, replacement, and directory-sync failures run best-effort cleanup; combined primary/cleanup errors expose only operation and I/O classes. Failpoint coverage confirms no temporary config remains.

## Verification

- `cargo fmt --check` — passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` — passed.
- `cargo test --locked --all-features` — passed, 42 tests.
- `npm --prefix web run typecheck` — passed.
- `npm --prefix web run test:ci` — passed, 34 tests.
- `cargo build --release --locked` — passed.
- `git diff --check` — passed.
- `cargo +1.94.0 check --locked --all-targets --all-features` on Linux — passed.
- Local `cargo +1.94.0 check --locked --target x86_64-pc-windows-msvc` reached Windows dependencies but could not finish on Linux because the host lacks MSVC tools (`ml64.exe`/`lib.exe`). The blocking `windows-latest` CI job runs the exact required command.

## Changed files

- Authentication/API: `src/api/rate_limit.rs`, `src/api/routes.rs`, `src/app.rs`, `tests/setup_auth_api.rs`.
- Setup/bootstrap/email: `src/setup/service.rs`, `src/main.rs`, `src/auth/users.rs`.
- Windows/CI/docs: `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`, `docs/configuration.md`.

## Self-review and concerns

- Rechecked the permit lifetime through account delay, database lookup, Argon2, session creation, and throttle recording; constants are exactly 4,096 attempts, 15 minutes, and four permits.
- Rechecked durable ordering and failure state: no administrator transaction begins before the supported platform replacement completes, and directory-sync failure leaves zero users and no temporary config.
- Rechecked bootstrap races: an `AlreadyClaimed` result becomes `READY` only after the authoritative users-plus-claim classification reports a consistent ready database.
- No known code concern remains. Windows cfg compilation is delegated to the added native Windows CI job because local MSVC build tools are unavailable.

Commit SHA: this report is part of the same atomic commit; the final SHA is reported in the handoff because a commit cannot contain its own hash.
