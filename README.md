# VAMOS (VaultScope API Management & Operations System)

VAMOS is the core backend engine powering the VaultScope ecosystem, written in highly-performant Rust using the Axum framework and SQLx.

## Architecture & Features
- **Centralized API**: Serves both the customer Storefront (`VaultScope`) and the staff Admin panel (`VaultScope-Admin`).
- **Provisioning Engine**: Includes an async `JobRunner` daemon that securely connects to upstream providers (e.g., Hetzner Cloud) to automatically provision hardware.
- **RBAC & Security**: 
  - Role-Based Access Control for administrative endpoints.
  - Strict Axum-based `axum_csrf` token validation.
  - Secure AES-256-GCM encryption for storing provider credentials and API tokens.
  - Rate limiting against brute-force attacks.
- **PostgreSQL**: Robust relational data modeling for customers, products, jobs, and invoices.

## Getting Started

### Prerequisites
- Rust & Cargo
- PostgreSQL (ensure you run the migrations in `crates/db/migrations`)

### Installation & Run
```bash
# Create the .env file with your DATABASE_URL
cp .env.example .env

# Run the API and background workers
cargo run
```

## Testing & CI
E2E tests and standard Rust `cargo test` workflows run automatically via GitHub actions.

## License
See the [LICENSE](LICENSE) file for details.