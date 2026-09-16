# Frontend. Built here so the image carries one artifact: API and UI, one origin.
FROM node:24-bookworm-slim AS web

WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-fund --no-audit
COPY web/ ./
RUN npm run build


FROM rust:1.98-slim-bookworm AS build

# The deployment target is an i7-7700 (Kaby Lake), so x86-64-v3 is available:
# SSE4.2, AVX, AVX2, BMI1/2, FMA, POPCNT, LZCNT. That is every x86-64 CPU from
# Haswell (2013) onward, so the image stays portable across the fleet while still
# using the vector width this box actually has.
#
# Override for an older host:  docker build --build-arg TARGET_CPU=x86-64-v2
# Fully portable baseline:     docker build --build-arg TARGET_CPU=x86-64
ARG TARGET_CPU=x86-64-v3

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    RUSTFLAGS="-C target-cpu=${TARGET_CPU}" cargo build --release \
    && install -Dm755 target/release/algo_index_engine /out/algo_index_engine


FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin blackbox

COPY --from=build /out/algo_index_engine /usr/local/bin/algo_index_engine
COPY --from=web /web/dist /app/web/dist

ENV BLACKBOX_HOME=/app \
    TZ=Asia/Kolkata

WORKDIR /app
RUN mkdir -p /app/data && chown -R blackbox:blackbox /app

USER blackbox

ENTRYPOINT ["/usr/local/bin/algo_index_engine"]
