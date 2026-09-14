# Getting Started with VAMOS

This guide will help you set up the VAMOS API from scratch.

## Prerequisites

### Required Software

1. **Rust 1.75+**
   ```bash
   # Install via rustup
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   
   # Verify installation
   rustc --version
   cargo --version
   ```

2. **PostgreSQL 14+**
   ```bash
   # macOS
   brew install postgresql@14
   
   # Ubuntu/Debian
   sudo apt install postgresql-14
   
   # Start service
   sudo systemctl start postgresql
   ```

3. **Redis 6+**
   ```bash
   # macOS
   brew install redis
   
   # Ubuntu/Debian
   sudo apt install redis-server
   
   # Start service
   redis-server
   ```

4. **SQLx CLI** (for migrations)
   ```bash
   cargo install sqlx-cli --no-default-features --features postgres
   ```

### Required Services

- **Authentik**: OIDC provider (see Authentik setup guide)

## Installation

### 1. Clone Repository

```bash
git clone https://github.com/VaultScope/vamos.git
cd vamos
```

### 2. Configure Environment

```bash
cp .env.example .env
```

Edit `.env` and configure:

```bash
# Database
DATABASE_URL=postgres://vaultscope:password@localhost:5432/vaultscope

# Generate secrets
JWT_SECRET=$(openssl rand -base64 64)
ENCRYPTION_KEY=$(openssl rand -base64 32)

# Authentik (configure in your Authentik instance first)
AUTHENTIK_ISSUER=https://auth.vaultscope.de
AUTHENTIK_CLIENT_ID_ADMIN=vaultscope-admin
AUTHENTIK_CLIENT_SECRET_ADMIN=<from-authentik>
AUTHENTIK_CLIENT_ID_STOREFRONT=vaultscope-storefront
AUTHENTIK_CLIENT_SECRET_STOREFRONT=<from-authentik>

# CORS
CORS_ORIGINS=http://localhost:5173,http://localhost:5174

# Redis
REDIS_URL=redis://localhost:6379
```

### 3. Setup Database

```bash
# Create database
createdb vaultscope

# Or via SQL
psql -U postgres -c "CREATE DATABASE vaultscope;"

# Run migrations
cd crates/db
sqlx database create
sqlx migrate run
cd ../..
```

### 4. Build and Run

```bash
# Development mode
cargo run

# With auto-reload (install cargo-watch first)
cargo install cargo-watch
cargo watch -x run

# With debug logging
RUST_LOG=debug cargo run
```

Server starts at `http://localhost:3000`

## Verify Installation

```bash
# Health check
curl http://localhost:3000/api/health

# Should return: {"status":"ok"}
```

## Development Workflow

### Running Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p vaultscope-api

# With output
cargo test -- --nocapture

# Integration tests
cargo test --test '*'
```

### Code Quality

```bash
# Linting
cargo clippy

# Format check
cargo fmt --check

# Format code
cargo fmt

# Check compilation
cargo check
```

### Database Migrations

```bash
# Create new migration
sqlx migrate add <migration_name>

# Run migrations
sqlx migrate run

# Revert last migration
sqlx migrate revert
```

## Common Issues

### Port Already in Use

```bash
# Find process
lsof -i :3000

# Kill process
kill -9 <PID>

# Or change port in .env
PORT=3001
```

### Database Connection Failed

1. Verify PostgreSQL is running:
   ```bash
   psql -U postgres -c "SELECT version();"
   ```

2. Check DATABASE_URL format
3. Ensure database exists: `psql -l`

### Redis Connection Failed

```bash
# Check Redis is running
redis-cli ping

# Should return: PONG
```

### Compilation Errors

```bash
# Clean and rebuild
cargo clean
cargo build
```

## Next Steps

- Read [ARCHITECTURE.md](ARCHITECTURE.md) to understand the system
- Review [API_REFERENCE.md](API_REFERENCE.md) for endpoint documentation
- See [DATABASE.md](DATABASE.md) for schema details
- Check [SECURITY.md](SECURITY.md) for security practices
