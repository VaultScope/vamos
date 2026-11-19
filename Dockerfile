FROM rust:1-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY crates ./crates

ENV SQLX_OFFLINE=true
RUN cargo build --locked --release --bin vaultscope-api

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates wget && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 10001 vaultscope && \
    useradd --system --uid 10001 --gid vaultscope --create-home --home-dir /app vaultscope

WORKDIR /app
RUN mkdir -p /app/uploads/avatars /app/uploads/tickets /app/uploads/blog /app/uploads/media && \
    chown -R vaultscope:vaultscope /app

COPY --from=builder /app/target/release/vaultscope-api /usr/local/bin/vaultscope-api

USER vaultscope

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --retries=3 --start-period=40s \
    CMD wget --spider -q http://localhost:3000/api/health || exit 1

CMD ["vaultscope-api"]
