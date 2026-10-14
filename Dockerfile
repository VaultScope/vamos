FROM rust:1-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY crates ./crates

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin vaultscope-api

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates wget && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
RUN mkdir -p /app/uploads/avatars /app/uploads/tickets /app/uploads/blog /app/uploads/media

COPY --from=builder /app/target/release/vaultscope-api /usr/local/bin/vaultscope-api

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --retries=3 --start-period=40s \
    CMD wget --spider -q http://localhost:3000/api/health || exit 1

CMD ["vaultscope-api"]
