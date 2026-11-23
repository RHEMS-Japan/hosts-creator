# syntax=docker/dockerfile:1

# ---- build ----
FROM rust:1-bookworm AS builder

# aws-lc-sys のビルドに cmake が必要
RUN apt-get update \
 && apt-get install -y --no-install-recommends cmake \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./

# 依存クレートだけ先にビルドしてレイヤキャッシュに乗せる
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && : > src/lib.rs \
 && cargo build --release --locked

COPY src ./src
RUN touch src/main.rs src/lib.rs \
 && cargo build --release --locked \
 && strip target/release/hosts-creator

# ---- runtime ----
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /etc/create_hosts

COPY --from=builder /src/target/release/hosts-creator /usr/local/bin/hosts-creator

WORKDIR /etc/create_hosts
ENTRYPOINT ["/usr/local/bin/hosts-creator"]
