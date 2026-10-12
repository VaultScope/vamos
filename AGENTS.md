# VaultScope API

## Verified Commands
- Install/Check: `cargo check`
- Build: `cargo build`
- Lint: `cargo clippy -- -D warnings` (Note: currently fails due to unused `verify_legacy` in `jwt.rs`)
- Test: `cargo test` (Note: currently fails due to missing `vaultscope_test` database)

## Invariants & Conventions
- Rust 2021 Edition workspace.
- Axum framework for HTTP routing.
- SQLx for database queries and migrations. Database pool injected via `State<AppState>`.
- JWT Audience enforcement: `Audience::Storefront` and `Audience::Admin` strictly separate access.

## Interfaces to Ecosystem
- **Storefront**: Mounted at `/api/storefront`.
- **Admin**: Mounted at `/api/admin`.
- **Database**: Sole owner of the Postgres schema. No other system touches the DB.

## No-Go Areas & Pitfalls
- **dev_login**: The `/api/auth/dev-login` route does NOT check for environment, posing a critical security risk.
- **Roles**: Admin access requires `require_permission!` checks against the `roles` table.
- **Dead Code**: Clippy will fail CI if unused functions exist (e.g. `verify_legacy`).
- **Tests**: Require an active, seeded Postgres database (`vaultscope_test`).

## Detail Docs
- [API Contract](/CONTRACT_MISMATCHES.md)
- [Auth Flow](docs/auth.md)
