# syntax=docker/dockerfile:1
FROM rust:1.85-bookworm AS build

WORKDIR /workspace
RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,id=switchyard-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=switchyard-jas-base-target,target=/workspace/target,sharing=locked \
    cargo build --locked --release --package switchyard-router \
    && cp target/release/switchyard-router /tmp/switchyard-router \
    && rustc --edition=2024 -O examples/jas-base/fixture.rs -o /tmp/jas-base-fixture

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libssl3 netcat-openbsd wget \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/switchyard-router /usr/local/bin/switchyard-router
COPY --from=build /tmp/jas-base-fixture /usr/local/bin/jas-base-fixture
COPY examples/jas-base/process-compose /usr/local/bin/process-compose
RUN chmod 0755 /usr/local/bin/process-compose \
    && install -d -o 65532 -g 65532 /data

USER 65532:65532
