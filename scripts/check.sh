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
    echo "cargo-audit is required: cargo install cargo-audit --locked" >&2
    return 1
  fi
  cargo audit
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
