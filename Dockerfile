# eduquery — сборка в одном образе.
#
#   docker build -t eduquery:latest .
#
# Два этапа: Rust с llama.cpp (нужен clang для bindgen) → runtime с Alpine.

FROM --platform=$BUILDPLATFORM rust:1.83-alpine AS build

# Зависимости для сборки llama.cpp: cmake, C/C++ компилятор, clang (bindgen),
# musl-dev — иначе не соберётся нативная часть llama.cpp.
RUN apk add --no-cache \
        cmake \
        clang \
        clang-dev \
        build-base \
        musl-dev \
        linux-headers \
        pkgconfig \
        git

WORKDIR /src

# Сначала зависимости — слой кэшируется между сборками.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps target/release/build \
    && rm -f target/release/eduquery

COPY src ./src
COPY web ./web

ARG VERSION=0.1.0
RUN cargo build --release --locked \
    && strip target/release/eduquery || true

# ── Runtime ──
FROM alpine:3.20

RUN apk add --no-cache \
        ca-certificates \
        libgomp \
        tzdata \
        curl

# Статический бинарь + веб-интерфейс.
COPY --from=build /src/target/release/eduquery /usr/local/bin/eduquery
COPY --from=build /src/web /opt/eduquery/web

# Каталог данных: БД и скачанные модели живут здесь.
ENV EDUQUERY_DATA_DIR=/data \
    EDUQUERY_HOST=0.0.0.0 \
    EDUQUERY_PORT=8080
VOLUME ["/data"]
WORKDIR /opt/eduquery

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/eduquery"]
