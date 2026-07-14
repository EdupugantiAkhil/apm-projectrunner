# syntax=docker/dockerfile:1
FROM rust:1.85-bookworm AS build

WORKDIR /workspace
RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,id=switchyard-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=switchyard-routing-target,target=/workspace/target,sharing=locked \
    cargo build --locked --release --package switchyard-router \
    && cp target/release/switchyard-router /tmp/switchyard-router \
    && rustc --edition=2024 -O examples/routing-matrix/fixture.rs -o /tmp/routing-fixture

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/switchyard-router /usr/local/bin/switchyard-router
COPY --from=build /tmp/routing-fixture /usr/local/bin/routing-fixture
RUN install -d -o 65532 -g 65532 /data

USER 65532:65532
