# eduquery — лёгкий образ. llama.cpp сюда НЕ вшит: приложение само скачивает
# движок (llama-server) и модели из интернета в /data при первом запуске.
#
#   docker build -t eduquery:latest .

FROM rust:1-bookworm AS build
WORKDIR /src

# Сначала зависимости — слой кэшируется между сборками.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/eduquery* target/release/eduquery

COPY src ./src
COPY web ./web
RUN cargo build --release --locked

# ── Runtime ──
# bookworm: glibc совместим с готовыми сборками llama.cpp (ubuntu-x64/arm64).
FROM debian:bookworm-slim

# libgomp1/libstdc++6/libssl3 нужны скачиваемому llama-server.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl tzdata libgomp1 libstdc++6 libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/eduquery /usr/local/bin/eduquery
COPY --from=build /src/web /opt/eduquery/web

# /data: БД, модели и скачанный движок.
ENV EDUQUERY_DATA_DIR=/data \
    EDUQUERY_HOST=0.0.0.0 \
    EDUQUERY_PORT=8080
VOLUME ["/data"]
WORKDIR /opt/eduquery
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/eduquery"]
