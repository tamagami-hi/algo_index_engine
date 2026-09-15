FROM rust:1.98-slim-bookworm AS build

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release \
    && install -Dm755 target/release/algo_index_engine /out/algo_index_engine

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin blackbox

COPY --from=build /out/algo_index_engine /usr/local/bin/algo_index_engine

ENV BLACKBOX_HOME=/app \
    TZ=Asia/Kolkata

WORKDIR /app
RUN mkdir -p /app/data && chown -R blackbox:blackbox /app

USER blackbox

ENTRYPOINT ["/usr/local/bin/algo_index_engine"]
