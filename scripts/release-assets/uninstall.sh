#!/usr/bin/env bash
set -euo pipefail

prefix="${HOME}/.local"
if [[ $# -eq 2 && $1 == "--prefix" ]]; then
  prefix=$2
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--prefix <path>]" >&2
  exit 2
fi

command -v sha256sum >/dev/null || {
  echo "uninstall: sha256sum is required" >&2
  exit 1
}
require_safe_parent() {
  local relative=$1 current=$prefix part
  local -a parts
  [[ ! -L $current && -d $current ]] || {
    echo "uninstall: prefix is not a safe directory: $current" >&2
    exit 1
  }
  IFS=/ read -r -a parts <<< "${relative%/*}"
  for part in "${parts[@]}"; do
    current="$current/$part"
    [[ ! -L $current && -d $current ]] || {
      echo "uninstall: refusing unsafe destination directory $current" >&2
      exit 1
    }
  done
}
manifest="$prefix/share/switchyard/installed-files.manifest"
require_safe_parent "share/switchyard/installed-files.manifest"
[[ -f $manifest && ! -L $manifest ]] || {
  echo "uninstall: no owned installation manifest at $manifest" >&2
  exit 1
}

declare -a files=()
while read -r expected relative; do
  [[ -n ${expected:-} && -n ${relative:-} ]] || continue
  case "$relative" in
    /*|../*|*/../*|*/..) echo "uninstall: unsafe manifest path $relative" >&2; exit 1 ;;
  esac
  destination="$prefix/$relative"
  require_safe_parent "$relative"
  [[ -f $destination && ! -L $destination ]] || {
    echo "uninstall: owned file is missing or unsafe: $destination" >&2
    exit 1
  }
  actual=$(sha256sum "$destination" | awk '{print $1}')
  [[ $actual == "$expected" ]] || {
    echo "uninstall: refusing to remove modified file $destination" >&2
    exit 1
  }
  files+=("$destination")
done < "$manifest"

for file in "${files[@]}"; do
  rm -- "$file"
  echo "removed $file"
done
rm -- "$manifest"
echo "removed $manifest"
find "$prefix/share/switchyard/web" -depth -type d -empty -delete 2>/dev/null || true
rmdir "$prefix/share/switchyard" 2>/dev/null || true
rmdir "$prefix/share" 2>/dev/null || true
rmdir "$prefix/bin" 2>/dev/null || true
rmdir "$prefix" 2>/dev/null || true
