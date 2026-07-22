#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

for command in tar mktemp; do
  command -v "$command" >/dev/null || {
    echo "release smoke: $command is required" >&2
    exit 1
  }
done
if command -v sha256sum >/dev/null 2>&1; then
  verify_sha256() { sha256sum --check "$1"; }
elif command -v shasum >/dev/null 2>&1; then
  verify_sha256() { shasum -a 256 --check "$1"; }
else
  echo "release smoke: sha256sum or shasum is required" >&2
  exit 1
fi
if [[ ${1:-} == "--build" ]]; then
  ./scripts/release.sh
  shift
fi
[[ $# -eq 0 ]] || {
  echo "usage: $0 [--build]" >&2
  exit 2
}
[[ -f dist/SHA256SUMS ]] || {
  echo "release smoke: dist/SHA256SUMS is missing; run scripts/release.sh first" >&2
  exit 1
}
archives=(dist/switchyard-*.tar.gz)
[[ ${#archives[@]} -eq 1 && -f ${archives[0]} ]] || {
  echo "release smoke: expected exactly one dist/switchyard-*.tar.gz" >&2
  exit 1
}
(
  cd dist
  verify_sha256 SHA256SUMS
)

temporary=$(mktemp -d "${TMPDIR:-/tmp}/switchyard-release-smoke.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
tar -C "$temporary" -xzf "${archives[0]}"
roots=("$temporary"/*/)
[[ ${#roots[@]} -eq 1 && -d ${roots[0]} ]] || {
  echo "release smoke: archive must contain exactly one top-level directory" >&2
  exit 1
}
prefix="$temporary/prefix"
"${roots[0]}/install.sh" --prefix "$prefix"
"$prefix/bin/switchyard" --help >/dev/null
"$prefix/bin/switchyard-daemon" --version >/dev/null
router_help="$temporary/router-help.txt"
if ! "$prefix/bin/switchyard-router" --help >"$router_help" 2>&1; then
  grep -q 'usage:' "$router_help"
fi
"${roots[0]}/install.sh" --prefix "$prefix"
"$prefix/bin/switchyard-uninstall" --prefix "$prefix"
[[ ! -e $prefix ]] || {
  echo "release smoke: uninstall left files under $prefix" >&2
  find "$prefix" -mindepth 1 -print >&2
  exit 1
}
echo "release smoke: checksum, install, in-place upgrade, executable invocation, ownership-checked uninstall, and clean prefix passed"
