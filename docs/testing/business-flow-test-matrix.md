# Business Flow Test Matrix

## Goal
After GUI-CLI-Core layering, business logic verification is core/CLI-first, with GUI focused on integration and UX wiring.

## Fast Local Gate (required before commit)
1. `cargo test -p clawpal-core`
2. `cargo test -p clawpal-cli`
3. `cargo build -p clawpal`

## Extended Local Gate (recommended before merge)
1. `cargo test -p clawpal --test install_api --test runtime_types --test commands_delegation`
2. `cargo run -p clawpal-cli -- instance list`
3. `cargo run -p clawpal-cli -- ssh list`
4. `cargo test -p clawpal --test wsl2_runner` (non-Windows host runs placeholder only)

## Remote Gate (requires reachable `vm1`)
1. `cargo test -p clawpal --test remote_api -- --test-threads=1`

Expected notes:
- 4 tests are `ignored` in `remote_api` by design (manual/optional checks).
- Environment must allow outbound SSH to `vm1`.

## Optional Live Docker Gate (local machine only)
1. `CLAWPAL_RUN_DOCKER_LIVE_TESTS=1 cargo test -p clawpal-core --test docker_live -- --nocapture`

Expected notes:
- If local port `18789` is occupied, the test will skip to avoid killing existing services.
- When port is free, test runs real `docker compose` workflow and then `down -v` cleanup.

## Optional WSL2 Gate (Windows only)
1. `cargo test -p clawpal --test wsl2_runner -- --ignored`

Expected notes:
- Requires WSL2 installed on host.
- `Install/Verify` cases depend on `openclaw` availability in WSL distribution.

## Layer Ownership
- `clawpal-core`: business rules, persistence, SSH registry, install/connect health logic.
- `clawpal-cli`: JSON contract and command routing.
- `src-tauri`: thin command delegation, state wiring, runtime event bridge.
- Frontend GUI: user interactions, rendering, invoke approval UX.

## Regression Priorities
1. Instance registry consistency (`instances.json` for local/docker/remote ssh).
2. SSH read/write correctness (must fail loudly on remote command errors).
3. Docker install behavior (no-op regressions blocked).
4. Doctor tool contract (`clawpal`/`openclaw` only).
