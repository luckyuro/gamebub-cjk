#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
font_version="2026.09.01"
font_name="fusion-pixel-12px-proportional-zh_hans.ttf"
expected_sha256="1b423de0be589d159ef71af7d00530a7176e16af768a26adbc2e524e8817aad9"
archive_name="fusion-pixel-font-12px-proportional-ttf-v${font_version}.zip"
release_url="https://github.com/TakWolf/fusion-pixel-font/releases/download/${font_version}/${archive_name}"
destination="$script_dir/res/fonts/$font_name"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

if [[ -f "$destination" ]] && [[ "$(sha256_file "$destination")" == "$expected_sha256" ]]; then
    echo "Fusion Pixel ${font_version} is already installed: $destination"
    exit 0
fi

for command_name in curl unzip; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: $command_name is required" >&2
        exit 1
    fi
done

temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gamebub-fusion-font.XXXXXX")"
trap 'rm -rf "$temp_dir"' EXIT

archive_path="$temp_dir/$archive_name"
extract_dir="$temp_dir/extracted"
mkdir -p "$extract_dir"

echo "Downloading Fusion Pixel ${font_version}..."
curl --fail --location --retry 3 --output "$archive_path" "$release_url"
unzip -q "$archive_path" -d "$extract_dir"

source_font="$(find "$extract_dir" -type f -name "$font_name" -print -quit)"
if [[ -z "$source_font" ]]; then
    echo "error: $font_name was not found in $archive_name" >&2
    exit 1
fi

actual_sha256="$(sha256_file "$source_font")"
if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "error: SHA-256 mismatch for $font_name" >&2
    echo "expected: $expected_sha256" >&2
    echo "actual:   $actual_sha256" >&2
    exit 1
fi

mkdir -p "$(dirname "$destination")"
cp "$source_font" "$destination"
echo "Installed $font_name to $destination"
