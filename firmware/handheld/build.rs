use regex::{Captures, Regex};
use std::{fs, path::PathBuf, process::Command};

const PACKED_FONT_ENV: &str = "GAMEBUB_PACK_FUSION_PIXEL";

fn strip_fusion_pixel_coverage_text() {
    let generated_path = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("main.rs");
    let mut generated =
        fs::read_to_string(&generated_path).expect("read generated Slint Rust code");

    // The hidden Text in fusion_pixel_full_coverage.slint is only an input to Slint's
    // compile-time glyph collector. Once the bitmap font has been generated, retaining
    // that 36,558-code-point string would waste flash and allocate it at UI start-up.
    let coverage_prefix =
        "sp :: SharedString :: from (\" !\\\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let value_start = generated
        .find(coverage_prefix)
        .expect("Fusion Pixel coverage text not found")
        + "sp :: SharedString :: from (".len();
    let value_end = generated[value_start..]
        .find("\")) as sp :: SharedString")
        .map(|offset| value_start + offset + 1)
        .expect("Fusion Pixel coverage text terminator not found");
    let removed_bytes = value_end - value_start;
    assert!(
        removed_bytes > 100_000,
        "Fusion Pixel coverage text was unexpectedly short"
    );
    generated.replace_range(value_start..value_end, "\"\"");
    fs::write(&generated_path, generated).expect("strip generated Fusion Pixel coverage text");
    println!("cargo:warning=stripped {removed_bytes} bytes of compile-only font coverage text");
}

fn pack_fusion_pixel_bitmaps() {
    let generated_path = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("main.rs");
    let generated = fs::read_to_string(&generated_path).expect("read generated Slint Rust code");

    let family_marker = "Fusion Pixel 12px Prop zh_hans";
    let family_offset = generated
        .rfind(family_marker)
        .expect("Fusion Pixel resource not generated");
    let resource_offset = generated[..family_offset]
        .rfind("static SLINT_EMBEDDED_RESOURCE_")
        .expect("Fusion Pixel resource declaration not found");
    let (prefix, font_resource) = generated.split_at(resource_offset);

    let data_array = Regex::new(r"(?s)static DATA : \[u8 ;\s*(\d+)usize\] = \[(.*?)\] ;").unwrap();
    let pixel_value = Regex::new(r"(\d+)u8").unwrap();
    let mut glyph_count = 0usize;
    let mut unpacked_bytes = 0usize;
    let mut packed_bytes = 0usize;

    let packed_resource = data_array.replace_all(font_resource, |captures: &Captures<'_>| {
        let declared_len: usize = captures[1].parse().unwrap();
        let pixels: Vec<u8> = pixel_value
            .captures_iter(&captures[2])
            .map(|pixel| pixel[1].parse().unwrap())
            .collect();
        assert_eq!(
            pixels.len(),
            declared_len,
            "unexpected Slint glyph array length"
        );
        assert!(
            pixels.iter().all(|pixel| matches!(pixel, 0 | 255)),
            "Fusion Pixel raster contains a non-binary alpha value"
        );

        let mut packed = vec![0u8; pixels.len().div_ceil(8)];
        for (index, pixel) in pixels.iter().enumerate() {
            if *pixel != 0 {
                packed[index / 8] |= 1 << (7 - index % 8);
            }
        }
        for (index, pixel) in pixels.iter().enumerate() {
            let unpacked = if packed[index / 8] & (1 << (7 - index % 8)) != 0 {
                255
            } else {
                0
            };
            assert_eq!(unpacked, *pixel, "Fusion Pixel bit-pack round-trip failed");
        }

        glyph_count += 1;
        unpacked_bytes += pixels.len();
        packed_bytes += packed.len();
        let values = packed
            .iter()
            .map(|value| format!("{value}u8"))
            .collect::<Vec<_>>()
            .join(" , ");
        format!(
            "static DATA : [u8 ; {}usize] = [{}] ;",
            packed.len(),
            values
        )
    });

    assert_eq!(glyph_count, 36_558, "unexpected Fusion Pixel glyph count");
    assert_eq!(
        unpacked_bytes, 4_202_706,
        "unexpected Fusion Pixel bitmap size"
    );
    fs::write(&generated_path, format!("{prefix}{packed_resource}"))
        .expect("write bit-packed Slint Rust code");
    println!(
        "cargo:warning=bit-packed Fusion Pixel: {glyph_count} glyphs, {unpacked_bytes} -> {packed_bytes} bytes"
    );
}

fn main() {
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-env-changed={PACKED_FONT_ENV}");

    slint_build::compile_with_config(
        "res/ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .unwrap();

    strip_fusion_pixel_coverage_text();

    if std::env::var_os(PACKED_FONT_ENV).is_some() {
        pack_fusion_pixel_bitmaps();
    }

    // Get git commit hash
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let output = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .expect("git command failed");
    let commit_hash = str::from_utf8(&output.stdout)
        .expect("git output invalid utf-8")
        .trim();
    println!("cargo:rustc-env=GIT_COMMIT={}", commit_hash);
}
