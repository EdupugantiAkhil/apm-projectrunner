#!/usr/bin/env bash
set -euo pipefail

package_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix="${HOME}/.local"
if [[ $# -eq 2 && $1 == "--prefix" ]]; then
  prefix=$2
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--prefix <path>]" >&2
  exit 2
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256_file() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  sha256_file() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  echo "install: sha256sum or shasum is required" >&2
  exit 1
fi

require_safe_parent() {
  local relative=$1 current=$prefix part
  local -a parts
  [[ ! -L $current && ( ! -e $current || -d $current ) ]] || {
    echo "install: prefix is not a safe directory: $current" >&2
    exit 1
  }
  IFS=/ read -r -a parts <<< "${relative%/*}"
  for part in "${parts[@]}"; do
    current="$current/$part"
    [[ ! -L $current && ( ! -e $current || -d $current ) ]] || {
      echo "install: refusing unsafe destination directory $current" >&2
      exit 1
    }
  done
}

manifest="$prefix/share/apmpr/installed-files.manifest"
require_safe_parent "share/apmpr/installed-files.manifest"
# Bash 3.2 treats a declared empty array as unbound under `set -u`. Keep an ignored
# sentinel so a first install can iterate safely on stock macOS Bash.
declare -a owned_hashes=("")
declare -a owned_relatives=("")
if [[ -e $manifest || -L $manifest ]]; then
  [[ -f $manifest && ! -L $manifest ]] || {
    echo "install: refusing non-regular manifest at $manifest" >&2
    exit 1
  }
  while read -r expected relative; do
    [[ -n ${expected:-} && -n ${relative:-} ]] || continue
    owned_hashes+=("$expected")
    owned_relatives+=("$relative")
  done < "$manifest"
fi

owned_hash_for() {
  local wanted=$1 index
  for index in "${!owned_relatives[@]}"; do
    if [[ ${owned_relatives[$index]} == "$wanted" ]]; then
      printf '%s\n' "${owned_hashes[$index]}"
      return 0
    fi
  done
  return 1
}

is_new_path() {
  local wanted=$1 candidate
  for candidate in "${relatives[@]}"; do
    [[ $candidate == "$wanted" ]] && return 0
  done
  return 1
}

declare -a sources=()
declare -a relatives=()
for binary in apmpr apmpr-daemon apmpr-router; do
  sources+=("$package_root/bin/$binary")
  relatives+=("bin/$binary")
done
sources+=("$package_root/uninstall.sh")
relatives+=("bin/apmpr-uninstall")
while IFS= read -r -d '' source; do
  sources+=("$source")
  relatives+=("share/apmpr/web/${source#"$package_root/web/"}")
done < <(find "$package_root/web" -type f -print0)

for index in "${!owned_relatives[@]}"; do
  relative=${owned_relatives[$index]}
  expected=${owned_hashes[$index]}
  [[ -n $relative ]] || continue
  case "$relative" in
    /*|../*|*/../*|*/..) echo "install: unsafe prior manifest path $relative" >&2; exit 1 ;;
  esac
  destination="$prefix/$relative"
  [[ -f $destination && ! -L $destination ]] || {
    echo "install: prior owned file is missing or unsafe: $destination" >&2
    exit 1
  }
  actual=$(sha256_file "$destination")
  [[ $actual == "$expected" ]] || {
    echo "install: prior owned file was modified: $destination" >&2
    exit 1
  }
done

for index in "${!sources[@]}"; do
  source=${sources[$index]}
  relative=${relatives[$index]}
  destination="$prefix/$relative"
  require_safe_parent "$relative"
  [[ -f $source && ! -L $source ]] || {
    echo "install: package file is missing or unsafe: $source" >&2
    exit 1
  }
  if [[ -e $destination || -L $destination ]]; then
    expected=$(owned_hash_for "$relative" || true)
    [[ -n $expected && -f $destination && ! -L $destination ]] || {
      echo "install: refusing to overwrite non-APM ProjectRunner file $destination" >&2
      exit 1
    }
    actual=$(sha256_file "$destination")
    [[ $actual == "$expected" ]] || {
      echo "install: refusing to overwrite modified APM ProjectRunner file $destination" >&2
      exit 1
    }
  fi
done

mkdir -p "$prefix/bin" "$prefix/share/apmpr/web"
temporary_manifest=$(mktemp "${TMPDIR:-/tmp}/apmpr-install.XXXXXX")
trap 'rm -f "$temporary_manifest"' EXIT
for index in "${!sources[@]}"; do
  source=${sources[$index]}
  relative=${relatives[$index]}
  destination="$prefix/$relative"
  mkdir -p "$(dirname "$destination")"
  install -m 0755 "$source" "$destination"
  if [[ $relative == share/apmpr/web/* ]]; then
    chmod 0644 "$destination"
  fi
  printf '%s  %s\n' "$(sha256_file "$destination")" "$relative" >> "$temporary_manifest"
  echo "installed $destination"
done
for relative in "${owned_relatives[@]}"; do
  [[ -n $relative ]] || continue
  if ! is_new_path "$relative"; then
    destination="$prefix/$relative"
    rm -- "$destination"
    echo "removed obsolete $destination"
  fi
done
find "$prefix/share/apmpr/web" -depth -type d -empty -delete 2>/dev/null || true
install -m 0600 "$temporary_manifest" "$manifest"
echo "installed $manifest"
