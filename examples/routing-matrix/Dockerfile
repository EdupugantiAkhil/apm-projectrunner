# syntax=docker/dockerfile:1
FROM rust:1.88-bookworm AS build

WORKDIR /workspace
RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,id=apmpr-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=apmpr-routing-target,target=/workspace/target,sharing=locked \
    cargo build --locked --release --package apmpr-router \
    && cp target/release/apmpr-router /tmp/apmpr-router \
    && rustc --edition=2024 -O examples/routing-matrix/fixture.rs -o /tmp/routing-fixture

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates iptables libssl3 wget \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/apmpr-router /usr/local/bin/apmpr-router
COPY --from=build /tmp/routing-fixture /usr/local/bin/routing-fixture
RUN install -d -o 65532 -g 65532 /data

USER 65532:65532
