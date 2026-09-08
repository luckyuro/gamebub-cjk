// Reuse the firmware's exact font stripping/packing code, including its assertions.
#[allow(dead_code)]
mod firmware_build {
    include!("../../build.rs");

    pub fn compile_host_ui() {
        println!("cargo:rerun-if-env-changed={PACKED_FONT_ENV}");
        slint_build::compile_with_config(
            "../../res/ui/main.slint",
            slint_build::CompilerConfiguration::new()
                .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
        )
        .unwrap();
        strip_fusion_pixel_coverage_text();
        if std::env::var_os(PACKED_FONT_ENV).is_some() {
            pack_fusion_pixel_bitmaps();
        }
    }
}

fn main() {
    firmware_build::compile_host_ui();
}
