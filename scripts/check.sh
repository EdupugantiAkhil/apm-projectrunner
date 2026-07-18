#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

run_fmt() {
  cargo fmt --all -- --check
}

run_lint() {
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

run_test() {
  cargo test --workspace --all-features
}

run_doc() {
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
}

run_audit() {
  if ! cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit is required: cargo install cargo-audit --locked --version 0.22.1" >&2
    return 1
  fi
  # Pingora 0.8.1 -> prometheus 0.13 -> protobuf 2.28. Switchyard only reaches
  # protobuf while encoding its own metrics, not while decoding untrusted data.
  # quick-xml 0.39 (RUSTSEC-2026-0194/0195) is reached only inside the
  # wayland-scanner PROC-MACRO at build time, parsing Wayland protocol XML
  # bundled in the crates themselves; no untrusted XML reaches it at runtime.
  cargo audit \
    --ignore RUSTSEC-2024-0437 \
    --ignore RUSTSEC-2026-0194 \
    --ignore RUSTSEC-2026-0195
}

case "${1:-all}" in
  all)
    run_fmt
    run_lint
    run_test
    run_doc
    ;;
  fmt) run_fmt ;;
  lint) run_lint ;;
  test) run_test ;;
  doc) run_doc ;;
  audit) run_audit ;;
  *)
    echo "usage: $0 [all|fmt|lint|test|doc|audit]" >&2
    exit 2
    ;;
esac
