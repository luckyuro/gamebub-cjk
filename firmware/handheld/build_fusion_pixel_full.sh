#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$project_dir"

cargo_home="${CARGO_HOME:-${HOME:?}/.cargo}"
core_source="$(find "$cargo_home/registry/src" -mindepth 2 -maxdepth 2 \
    -type d -name 'i-slint-core-1.12.1' -print -quit)"
if [[ -z "$core_source" ]]; then
    echo "i-slint-core 1.12.1 is not cached; run a normal cargo build first." >&2
    exit 1
fi

patched_core="$project_dir/target/fusion-pixel-packed/i-slint-core-1.12.1"
mkdir -p "$patched_core"
cp -R "$core_source/." "$patched_core/"

# esp-idf-sys invokes `cargo metadata --locked` from its build script. Cargo does
# not propagate command-line --config values to that nested Cargo invocation, so
# put the Slint patch in an isolated CARGO_HOME that both Cargo processes see.
packed_cargo_home="$project_dir/target/fusion-pixel-packed/cargo-home"
mkdir -p "$packed_cargo_home"
for cache_dir in git registry; do
    if [[ -d "$cargo_home/$cache_dir" && ! -e "$packed_cargo_home/$cache_dir" ]]; then
        ln -s "$cargo_home/$cache_dir" "$packed_cargo_home/$cache_dir"
    fi
done
python3 - "$packed_cargo_home/config.toml" "$patched_core" <<'PY'
from pathlib import Path
import json
import sys

Path(sys.argv[1]).write_text(
    "[patch.crates-io]\n"
    f"i-slint-core = {{ path = {json.dumps(sys.argv[2])} }}\n"
)
PY

patch -d "$patched_core" -p1 < \
    "$project_dir/patches/i-slint-core-1.12.1-bitpacked-alpha.patch"

if [[ "$#" -eq 0 ]]; then
    set -- --release --features=rev4
fi

CARGO_HOME="$packed_cargo_home" GAMEBUB_PACK_FUSION_PIXEL=1 cargo build "$@"
