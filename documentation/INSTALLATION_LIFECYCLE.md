# VAMOS (VaultScope-API) — Lifecycle Guide
## Installation, Upgrading, and Uninstallation

This document provides complete instructions for installing, upgrading, and uninstalling VAMOS (VaultScope API Management & Operations System), the core backend engine of the VaultScope ecosystem.

---

## 1. Overview & Architecture

VAMOS is an asynchronous, high-performance API backend written in Rust utilizing:
- **Axum 0.8**: High-performance HTTP server routing and middleware
- **SQLx 0.8**: Compile-time verified PostgreSQL connection pooling and migrations
- **Tokio**: Asynchronous event loop and runtime
- **Redis**: Session token cache and token-bucket rate limiting
- **AES-256-GCM**: Symmetric encryption for upstream cloud provider credentials
- **JobRunner**: Background worker daemon handling provisioning jobs (VPS, Dedicated, ODP)

### Default Ports & Networking
- **Internal Port**: `3000`
- **Host Port**: `3000` (standalone) or `8000` (full-stack dev)
- **Health Check Endpoint**: `GET /api/health` (returns `{"status":"ok"}`)

---

## 2. Prerequisites & System Requirements

### Production Requirements
- **OS**: Linux (Ubuntu 20.04+, Debian 11+, RHEL 8+)
- **CPU**: 2+ cores recommended
- **RAM**: 2GB+ available RAM
- **PostgreSQL**: Version 14 or higher (16 recommended)
- **Redis**: Version 6 or higher (7 recommended)
- **OpenSSL**: libssl-dev / OpenSSL 3.x
- **Authentik**: OIDC provider with configured client credentials

### Bare-Metal Development Requirements
- **Rust Toolchain**: 1.75+ (`rustup default stable`)
- **SQLx CLI**: `cargo install sqlx-cli --no-default-features --features postgres`

---

## 3. Installation Methods

### Method A: Automated Full-Stack Installation (Production)
Run the root production installer from the host:
```bash
curl -fsSL https://raw.githubusercontent.com/VaultScope/website/dev/installer.sh | sudo bash
```
The installer automatically:
1. Clones VAMOS into `/opt/vaultscope/vamos`.
2. Generates a 64-byte base64 `JWT_SECRET` and 32-byte base64 `ENCRYPTION_KEY`.
3. Sets up PostgreSQL and Redis Docker containers.
4. Executes database migrations.
5. Builds and launches the `vaultscope-api` container with health checks and systemd management.

---

### Method B: Standalone Docker Compose Deployment
```bash
# 1. Clone the repository
git clone https://github.com/VaultScope/vamos.git
cd vamos

# 2. Configure environment file
cp .env.example .env

# Generate required secrets:
# JWT Secret (64-byte base64)
openssl rand -base64 64
# Encryption Key (32-byte base64)
openssl rand -base64 32

# 3. Build and launch services
docker-compose up -d --build

# 4. Verify API health
curl -f http://localhost:3000/api/health
```

---

### Method C: Bare-Metal Local Development
```bash
# 1. Clone repository
git clone https://github.com/VaultScope/vamos.git
cd vamos

# 2. Configure environment variables
cp .env.example .env
nano .env

# 3. Create database and apply migrations
createdb -U postgres vaultscope
cd crates/db
sqlx database create
sqlx migrate run
cd ../..

# 4. Run development server
RUST_LOG=info cargo run

# Or with live reload:
cargo install cargo-watch
cargo watch -x run
```

---

## 4. Configuration Reference (`.env`)

| Variable | Required | Default / Example | Description |
| :--- | :--- | :--- | :--- |
| `DATABASE_URL` | **Yes** | `postgres://vaultscope:pass@localhost:5432/vaultscope` | PostgreSQL connection string |
| `DB_MAX_CONNECTIONS` | No | `10` | Maximum connection pool size |
| `HOST` | No | `0.0.0.0` | Server listen host |
| `PORT` | No | `3000` | Server listen port |
| `JWT_SECRET` | **Yes** | *(64-byte base64 string)* | Cryptographic secret for signing session tokens |
| `JWT_EXPIRATION_SECS` | No | `3600` | Token expiration duration (seconds) |
| `ENCRYPTION_KEY` | **Yes** | *(32-byte base64 string)* | Key for AES-256-GCM provider credential encryption |
| `AUTHENTIK_ISSUER` | **Yes** | `https://auth.vaultscope.de` | Authentik OIDC discovery URL |
| `AUTHENTIK_CLIENT_ID_ADMIN` | **Yes** | `vaultscope-admin` | Admin OIDC client ID |
| `AUTHENTIK_CLIENT_SECRET_ADMIN` | **Yes** | *(secret string)* | Admin OIDC client secret |
| `AUTHENTIK_CLIENT_ID_STOREFRONT`| **Yes** | `vaultscope-storefront` | Storefront OIDC client ID |
| `AUTHENTIK_CLIENT_SECRET_STOREFRONT` | **Yes** | *(secret string)* | Storefront OIDC client secret |
| `CORS_ORIGINS` | **Yes** | `https://vaultscope.de,https://admin.vaultscope.de` | Comma-separated allowed CORS origins |
| `REDIS_URL` | **Yes** | `redis://localhost:6379` | Redis connection URL |
| `JOB_POLL_INTERVAL_SECS` | No | `5` | Provisioning queue poll interval (seconds) |
| `RATE_LIMIT_ANONYMOUS` | No | `60` | Public requests per minute limit |
| `RATE_LIMIT_AUTHENTICATED` | No | `300` | Authenticated requests per minute limit |
| `RUST_LOG` | No | `info` | Logging verbosity filter |

---

## 5. Verification & Health Checks

```bash
# 1. Check HTTP health endpoint
curl -s http://localhost:3000/api/health
# Expected output: {"status":"ok"}

# 2. Check container health (Docker)
docker inspect --format='{{json .State.Health.Status}}' vaultscope-api
# Expected output: "healthy"

# 3. Test PostgreSQL connectivity
docker exec -it vaultscope-postgres pg_isready -U vaultscope
```

---

## 6. Upgrading Procedures

### Automated Upgrades (`update.sh`)
```bash
sudo /opt/vaultscope/update.sh --branch=dev
```
The updater automatically:
1. Creates a full backup snapshot in `/opt/vaultscope/backups/`.
2. Fetches the latest Git commits.
3. Ensures database containers are running and applies pending SQLx migrations.
4. Rebuilds the Docker container with `--no-cache`.
5. Waits for health status verification before completing. If a failure occurs, the previous backup is automatically restored.

### Manual Container Upgrade
```bash
cd /opt/vaultscope
cd vamos && git pull origin dev && cd ..
docker-compose -f docker-compose.production.yml up -d postgres redis
docker-compose -f docker-compose.production.yml build --no-cache api
docker-compose -f docker-compose.production.yml up -d --no-deps api
```

### Bare-Metal Upgrade
```bash
cd vamos
git pull origin dev
cd crates/db && sqlx migrate run && cd ../..
cargo build --release
```

---

## 7. Uninstallation Procedures

### Complete System Uninstallation
```bash
sudo /opt/vaultscope/uninstall.sh
```

### Standalone Manual Teardown
```bash
# 1. Stop and remove Docker services
docker stop vaultscope-api vaultscope-postgres vaultscope-redis
docker rm -v vaultscope-api vaultscope-postgres vaultscope-redis

# 2. Remove Nginx site configuration
sudo rm -f /etc/nginx/sites-enabled/api.your-domain.com
sudo rm -f /etc/nginx/sites-available/api.your-domain.com
sudo systemctl reload nginx

# 3. Delete application code
sudo rm -rf /opt/vaultscope/vamos
```
