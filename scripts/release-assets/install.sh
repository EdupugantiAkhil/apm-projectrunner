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

command -v sha256sum >/dev/null || {
  echo "install: sha256sum is required" >&2
  exit 1
}

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

manifest="$prefix/share/switchyard/installed-files.manifest"
require_safe_parent "share/switchyard/installed-files.manifest"
declare -A owned_hashes=()
if [[ -e $manifest || -L $manifest ]]; then
  [[ -f $manifest && ! -L $manifest ]] || {
    echo "install: refusing non-regular manifest at $manifest" >&2
    exit 1
  }
  while read -r expected relative; do
    [[ -n ${expected:-} && -n ${relative:-} ]] || continue
    owned_hashes["$relative"]=$expected
  done < "$manifest"
fi

declare -a sources=()
declare -a relatives=()
declare -A new_paths=()
for binary in switchyard switchyard-daemon switchyard-router; do
  sources+=("$package_root/bin/$binary")
  relatives+=("bin/$binary")
done
sources+=("$package_root/uninstall.sh")
relatives+=("bin/switchyard-uninstall")
while IFS= read -r -d '' source; do
  sources+=("$source")
  relatives+=("share/switchyard/web/${source#"$package_root/web/"}")
done < <(find "$package_root/web" -type f -print0 | sort -z)

for relative in "${relatives[@]}"; do
  new_paths["$relative"]=1
done
for relative in "${!owned_hashes[@]}"; do
  case "$relative" in
    /*|../*|*/../*|*/..) echo "install: unsafe prior manifest path $relative" >&2; exit 1 ;;
  esac
  destination="$prefix/$relative"
  [[ -f $destination && ! -L $destination ]] || {
    echo "install: prior owned file is missing or unsafe: $destination" >&2
    exit 1
  }
  actual=$(sha256sum "$destination" | awk '{print $1}')
  [[ $actual == "${owned_hashes[$relative]}" ]] || {
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
    expected=${owned_hashes[$relative]:-}
    [[ -n $expected && -f $destination && ! -L $destination ]] || {
      echo "install: refusing to overwrite non-Switchyard file $destination" >&2
      exit 1
    }
    actual=$(sha256sum "$destination" | awk '{print $1}')
    [[ $actual == "$expected" ]] || {
      echo "install: refusing to overwrite modified Switchyard file $destination" >&2
      exit 1
    }
  fi
done

mkdir -p "$prefix/bin" "$prefix/share/switchyard/web"
temporary_manifest=$(mktemp "${TMPDIR:-/tmp}/switchyard-install.XXXXXX")
trap 'rm -f "$temporary_manifest"' EXIT
for index in "${!sources[@]}"; do
  source=${sources[$index]}
  relative=${relatives[$index]}
  destination="$prefix/$relative"
  mkdir -p "$(dirname "$destination")"
  install -m 0755 "$source" "$destination"
  if [[ $relative == share/switchyard/web/* ]]; then
    chmod 0644 "$destination"
  fi
  sha256sum "$destination" | awk -v path="$relative" '{print $1 "  " path}' >> "$temporary_manifest"
  echo "installed $destination"
done
for relative in "${!owned_hashes[@]}"; do
  if [[ -z ${new_paths[$relative]:-} ]]; then
    destination="$prefix/$relative"
    rm -- "$destination"
    echo "removed obsolete $destination"
  fi
done
find "$prefix/share/switchyard/web" -depth -type d -empty -delete 2>/dev/null || true
install -m 0600 "$temporary_manifest" "$manifest"
echo "installed $manifest"
