#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
source_file="$repo_root/web_ui/vendor/assistant-ui.SOURCE.json"
target_dir="$repo_root/web_ui/vendor/assistant-ui"

if [[ ! -f "$source_file" ]]; then
  echo "missing assistant-ui source pin: $source_file" >&2
  exit 1
fi

commit="$(sed -n 's/.*"commit": "\([0-9a-f]*\)".*/\1/p' "$source_file")"
if [[ ! "$commit" =~ ^[0-9a-f]{40}$ ]]; then
  echo "invalid assistant-ui commit pin in $source_file" >&2
  exit 1
fi

marker_file="$target_dir/.timem-source-pin"
if [[ -f "$target_dir/package.json" && -f "$marker_file" ]]; then
  installed_commit="$(<"$marker_file")"
  if [[ "$installed_commit" == "$commit" ]]; then
    echo "assistant-ui source already present: $target_dir ($commit)"
    exit 0
  fi
fi

if [[ -e "$target_dir" ]]; then
  echo "assistant-ui source exists but does not match the pinned revision: $target_dir" >&2
  echo "remove it, then rerun this script" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
archive="$tmp_dir/assistant-ui.tar.gz"

curl --fail --location --retry 3 --retry-delay 2 \
  "https://codeload.github.com/assistant-ui/assistant-ui/tar.gz/$commit" \
  --output "$archive"
tar -tzf "$archive" >/dev/null
tar -xzf "$archive" -C "$tmp_dir"

extracted_dir="$tmp_dir/assistant-ui-$commit"
if [[ ! -f "$extracted_dir/package.json" ]]; then
  echo "assistant-ui archive did not contain the expected source root" >&2
  exit 1
fi

mkdir -p "$(dirname "$target_dir")"
mv "$extracted_dir" "$target_dir"
printf '%s\n' "$commit" > "$marker_file"
echo "assistant-ui source ready at $target_dir ($commit)"
