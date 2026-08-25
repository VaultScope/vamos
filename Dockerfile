FROM rust:1-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin vaultscope-api

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/vaultscope-api /usr/local/bin/vaultscope-api

EXPOSE 3000

CMD ["vaultscope-api"]
