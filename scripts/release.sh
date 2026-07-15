#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

for command in cargo node npm git tar sha256sum install find sort awk sed uname; do
  command -v "$command" >/dev/null || {
    echo "release: $command is required" >&2
    exit 1
  }
done
node_major=$(node -p 'process.versions.node.split(".")[0]')
[[ $node_major == 24 ]] || {
  echo "release: Node.js 24 is required to build packages/web (found $(node --version))" >&2
  exit 1
}

cargo_version=$(awk '
  /^\[workspace.package\]$/ { in_package=1; next }
  /^\[/ { in_package=0 }
  in_package && /^version = / { gsub(/[" ]/, "", $3); print $3; exit }
' Cargo.toml)
[[ -n $cargo_version ]] || {
  echo "release: could not read workspace package version" >&2
  exit 1
}
git_describe=$(git describe --always --dirty)
release_version="${cargo_version}+${git_describe//\//-}"
os=$(uname -s | tr '[:upper:]' '[:lower:]')
machine=$(uname -m)
case "$machine" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) arch=$machine ;;
esac
archive_base="switchyard-${release_version}-${os}-${arch}"

cargo build --release \
  -p switchyard-cli \
  -p switchyard-daemon \
  -p switchyard-router
(
  cd packages/web
  npm ci
  npm run build
)
[[ -f packages/web/dist/index.html ]] || {
  echo "release: GUI build did not produce packages/web/dist/index.html" >&2
  exit 1
}

rm -rf dist
mkdir -p "dist/$archive_base/bin" "dist/$archive_base/web"
for binary in switchyard switchyard-daemon switchyard-router; do
  install -m 0755 "target/release/$binary" "dist/$archive_base/bin/$binary"
done
cp -R packages/web/dist/. "dist/$archive_base/web/"
install -m 0755 scripts/release-assets/install.sh "dist/$archive_base/install.sh"
install -m 0755 scripts/release-assets/uninstall.sh "dist/$archive_base/uninstall.sh"
for license in LICENSE LICENSE.txt LICENSE.md NOTICE NOTICE.txt NOTICE.md; do
  if [[ -f $license ]]; then
    install -m 0644 "$license" "dist/$archive_base/$license"
  fi
done

tar -C dist -czf "dist/$archive_base.tar.gz" "$archive_base"
rm -rf "dist/$archive_base"

previous_tag=$(git describe --tags --abbrev=0 2>/dev/null || true)
if [[ -n $previous_tag ]]; then
  changelog=$(git log --oneline "${previous_tag}..HEAD")
  [[ -n $changelog ]] || changelog="- No commits since ${previous_tag}."
else
  changelog=$(git log --oneline)
  [[ -n $changelog ]] || changelog="- No commits recorded."
fi
changelog=$(printf '%s\n' "$changelog" | sed 's/^/- `/; s/$/`/')
sed \
  -e "s|{{VERSION}}|$release_version|g" \
  -e "s|{{OS}}|$os|g" \
  -e "s|{{ARCH}}|$arch|g" \
  -e "/{{CHANGELOG}}/r /dev/stdin" \
  -e "/{{CHANGELOG}}/d" \
  docs/release-notes-template.md > dist/RELEASE_NOTES.md <<< "$changelog"

(
  cd dist
  sha256sum "$archive_base.tar.gz" RELEASE_NOTES.md > SHA256SUMS
)
if [[ -n ${SWITCHYARD_SIGNING_KEY:-} ]]; then
  command -v ssh-keygen >/dev/null || {
    echo "release: ssh-keygen is required when SWITCHYARD_SIGNING_KEY is set" >&2
    exit 1
  }
  [[ -f $SWITCHYARD_SIGNING_KEY ]] || {
    echo "release: SWITCHYARD_SIGNING_KEY does not name a private key file" >&2
    exit 1
  }
  rm -f dist/SHA256SUMS.sig
  ssh-keygen -Y sign -f "$SWITCHYARD_SIGNING_KEY" -n switchyard-release dist/SHA256SUMS
  echo "release: signed dist/SHA256SUMS"
else
  echo "release: unsigned release; set SWITCHYARD_SIGNING_KEY to an SSH private key to sign SHA256SUMS"
fi
echo "release: wrote dist/$archive_base.tar.gz, dist/RELEASE_NOTES.md, and dist/SHA256SUMS"
