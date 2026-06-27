# syncthing-rust Docker 镜像 (多阶段构建)
#
# 构建:  docker build -t syncthing-rust -f deploy/Dockerfile .
# 运行:  docker run -v /data/syncthing:/config -v /data/sync:/sync \
#           -p 8385:8385 -p 22000:22000 syncthing-rust run
#
# Relay: docker run -v /data/syncthing:/config \
#           -p 22067:22067 -p 22068:22068 \
#           syncthing-rust relay-server --listen 0.0.0.0:22067 --session-port 22068

# ── 构建阶段 ──────────────────────────────────────────────────
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY cmd/ cmd/

# 构建 release binary（仅 syncthing daemon/CLI，不含 tray）
RUN cargo build --release --bin syncthing --no-default-features \
    && strip target/release/syncthing

# ── 运行阶段 ──────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/syncthing /usr/local/bin/syncthing

# 健康检查
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD syncthing status || exit 1

VOLUME ["/config", "/sync"]
EXPOSE 8385 22000 22067 22068

ENTRYPOINT ["syncthing"]
CMD ["run"]
