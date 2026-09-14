# VAMOS (VaultScope API Management & Operations System)

[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)

The high-performance backend API powering the VaultScope ecosystem. Built with Rust, Axum, and SQLx, VAMOS provides secure API endpoints, automated provisioning, and robust job orchestration.

## 🌟 Features

- **🔐 Secure Authentication**: Authentik OIDC integration with JWT tokens
- **🏗️ Workspace Architecture**: Modular crate-based architecture (api, db, jobs, connectors, common)
- **⚡ Async Job Queue**: Background job runner for provisioning operations
- **🔒 Advanced Security**: RBAC, CSRF protection, rate limiting, AES-256-GCM encryption
- **🔗 Provider Connectors**: Automated infrastructure provisioning (Hetzner, OVH, etc.)
- **📊 PostgreSQL Backend**: Robust data modeling with SQLx compile-time checking
- **🚀 High Performance**: Async Rust with Tokio runtime

## 📋 Prerequisites

- **Rust**: 1.75 or higher
- **PostgreSQL**: 14 or higher
- **Redis**: 6 or higher (for OIDC state validation)
- **Authentik**: OIDC provider instance

## 🚀 Quick Start

```bash
# Clone and setup
git clone https://github.com/VaultScope/vamos.git
cd vamos
cp .env.example .env

# Edit .env with your DATABASE_URL and secrets
nano .env

# Run database migrations
cd crates/db && sqlx database create && sqlx migrate run && cd ../..

# Build and run
cargo run
```

Server starts at `http://localhost:3000`

## 📁 Project Structure

```
vamos/
├── crates/
│   ├── api/              # HTTP server (Axum)
│   ├── db/               # Database models & migrations
│   ├── jobs/             # Background job runner
│   ├── connectors/       # Provider integrations
│   └── common/           # Shared utilities
├── documentation/        # Comprehensive docs
├── Cargo.toml           # Workspace config
└── docker-compose.yml   # Dev services
```

## 🔧 Configuration

See `documentation/CONFIGURATION.md` for details.

### Required Environment Variables

```bash
DATABASE_URL=postgres://user:pass@localhost:5432/vaultscope
JWT_SECRET=<64+ byte secret>
ENCRYPTION_KEY=<32 byte key>
AUTHENTIK_ISSUER=https://auth.vaultscope.de
AUTHENTIK_CLIENT_ID_ADMIN=vaultscope-admin
AUTHENTIK_CLIENT_SECRET_ADMIN=<secret>
AUTHENTIK_CLIENT_ID_STOREFRONT=vaultscope-storefront
AUTHENTIK_CLIENT_SECRET_STOREFRONT=<secret>
REDIS_URL=redis://localhost:6379
CORS_ORIGINS=http://localhost:5173,http://localhost:5174
```

## 📚 Documentation

- **[Getting Started](documentation/GETTING_STARTED.md)** - Setup guide
- **[Architecture](documentation/ARCHITECTURE.md)** - System design
- **[API Reference](documentation/API_REFERENCE.md)** - Endpoint docs
- **[Database](documentation/DATABASE.md)** - Schema & migrations
- **[Security](documentation/SECURITY.md)** - Security architecture
- **[Deployment](documentation/DEPLOYMENT.md)** - Production deploy

## 🧪 Testing

```bash
cargo test                    # All tests
cargo test -p vaultscope-api  # Specific crate
cargo clippy                  # Linting
cargo fmt                     # Format check
```

## 📦 Production Build

```bash
cargo build --release
# Binary: ./target/release/vaultscope-api

# Docker
docker build -t vaultscope/vamos:latest .
docker-compose -f docker-compose.prod.yml up -d
```

## 🔗 API Endpoints

### Public
- `GET /api/health` - Health check
- `POST /api/auth/init-login` - Initialize OIDC
- `POST /api/auth/callback/{audience}` - OIDC callback

### Storefront (Authenticated)
- `GET /api/storefront/catalog` - Products
- `POST /api/storefront/orders` - Create order
- `GET /api/storefront/services` - User services

### Admin (Authenticated + RBAC)
- `GET /api/admin/dashboard` - Metrics
- `POST /api/admin/products` - Manage products
- `GET /api/admin/customers` - Customer list

See `documentation/API_REFERENCE.md` for complete docs.

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## 📄 License

AGPL-3.0 - see [LICENSE](LICENSE)

## 🔗 Related Projects

- **[website](https://github.com/VaultScope/website)** - Storefront (React)
- **[camos](https://github.com/VaultScope/camos)** - Admin Panel (React)

---

Built with 🦀 by the VaultScope team