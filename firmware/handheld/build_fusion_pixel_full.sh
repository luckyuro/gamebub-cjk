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

python3 - "$patched_core/software_renderer/fonts/pixelfont.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text()
old = """        Some(RenderableGlyph {
            x,
            y: h_plus_y - Fixed::from_integer(height),
            width: PhysicalLength::new(width as i16),
            height: PhysicalLength::new(height as i16),
            alpha_map: bitmap_glyph.data.as_slice().into(),
            pixel_stride: bitmap_glyph.width as u16,
            sdf: self.bitmap_font.sdf,
        })
"""
new = """        // The Game Bub full-Fusion-Pixel build stores binary alpha maps at one bit per
        // pixel. Expand only glyphs that are actually drawn; the scene retains the Rc for
        // the duration of the frame.
        let pixel_count = bitmap_glyph.width as usize * bitmap_glyph.height as usize;
        debug_assert_eq!(bitmap_glyph.data.len(), pixel_count.div_ceil(8));
        let alpha_map: alloc::rc::Rc<[u8]> = (0..pixel_count)
            .map(|index| {
                let byte = bitmap_glyph.data[index / 8];
                if byte & (1 << (7 - index % 8)) != 0 { 255 } else { 0 }
            })
            .collect::<alloc::vec::Vec<_>>()
            .into();
        Some(RenderableGlyph {
            x,
            y: h_plus_y - Fixed::from_integer(height),
            width: PhysicalLength::new(width as i16),
            height: PhysicalLength::new(height as i16),
            alpha_map: alpha_map.into(),
            pixel_stride: bitmap_glyph.width as u16,
            sdf: self.bitmap_font.sdf,
        })
"""
if old not in source:
    raise SystemExit("i-slint-core pixelfont implementation did not match 1.12.1")
path.write_text(source.replace(old, new, 1))
PY

if [[ "$#" -eq 0 ]]; then
    set -- --release --features=rev4
fi

CARGO_HOME="$packed_cargo_home" GAMEBUB_PACK_FUSION_PIXEL=1 cargo build "$@"
