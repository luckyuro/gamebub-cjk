use std::{
    io,
    path::{Path, PathBuf},
};

use crate::kvs::keys;

const SD_SYSTEM: &str = "/sdcard/system";
const BUILT_IN_SYSTEM: &str = "/system";

#[derive(Clone, Copy)]
pub enum Bios {
    Dmg,
    Cgb,
    Gba,
}

impl Bios {
    fn filename(self) -> &'static str {
        match self {
            Self::Dmg => "gameboy.bios-dmg.bin",
            Self::Cgb => "gameboy.bios-cgb.bin",
            Self::Gba if keys::GBA_SKIP_BOOT_ANIM.get().unwrap_or(false) => "gba.bios-fast.bin",
            Self::Gba => "gba.bios.bin",
        }
    }

    fn size(self) -> u64 {
        match self {
            Self::Dmg => 256,
            Self::Cgb => 2304,
            Self::Gba => 16 * 1024,
        }
    }

    pub fn use_sd_bios(self) -> bool {
        match self {
            Self::Dmg | Self::Cgb => keys::GB_USE_SD_BIOS.get().unwrap_or(true),
            Self::Gba => keys::GBA_USE_SD_BIOS.get().unwrap_or(true),
        }
    }

    /// Always load a BIOS: disabling the SD override selects the bundled replacement.
    pub fn path(self) -> PathBuf {
        resolve_path(
            Path::new(SD_SYSTEM),
            Path::new(BUILT_IN_SYSTEM),
            self.filename(),
            self.use_sd_bios(),
        )
    }

    pub fn sd_status(self) -> &'static str {
        file_status(&Path::new(SD_SYSTEM).join(self.filename()), self.size())
    }
}

fn resolve_path(sd: &Path, built_in: &Path, filename: &str, use_sd: bool) -> PathBuf {
    let sd_path = sd.join(filename);
    if use_sd && sd_path.is_file() {
        sd_path
    } else {
        built_in.join(filename)
    }
}

fn file_status(path: &Path, expected_size: u64) -> &'static str {
    match path.metadata() {
        Ok(metadata) if !metadata.is_file() => "Not found",
        Ok(metadata) if metadata.len() != expected_size => "Wrong size",
        Ok(_) => "Found",
        Err(error) if error.kind() == io::ErrorKind::NotFound => "Not found",
        Err(_) => "Read error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "gamebub-bios-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(path.join("sd")).unwrap();
            std::fs::create_dir_all(path.join("built-in")).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn bios_overrides_can_be_disabled_without_deleting_files() {
        let fixture = Fixture::new();
        let sd = fixture.0.join("sd");
        let built_in = fixture.0.join("built-in");
        for (filename, size) in [
            ("gameboy.bios-dmg.bin", 256),
            ("gameboy.bios-cgb.bin", 2304),
            ("gba.bios.bin", 16384),
            ("gba.bios-fast.bin", 16384),
        ] {
            std::fs::write(built_in.join(filename), vec![0x11; size]).unwrap();
            assert_eq!(
                resolve_path(&sd, &built_in, filename, true),
                built_in.join(filename)
            );
            assert_eq!(file_status(&sd.join(filename), size as u64), "Not found");

            std::fs::write(sd.join(filename), vec![0x22; size]).unwrap();
            for (use_sd, expected_byte) in [(true, 0x22), (false, 0x11), (true, 0x22)] {
                let selected = resolve_path(&sd, &built_in, filename, use_sd);
                assert!(std::fs::read(selected)
                    .unwrap()
                    .iter()
                    .all(|byte| *byte == expected_byte));
                // Detection is independent of the load preference.
                assert_eq!(file_status(&sd.join(filename), size as u64), "Found");
            }

            std::fs::remove_file(sd.join(filename)).unwrap();
            assert_eq!(
                resolve_path(&sd, &built_in, filename, true),
                built_in.join(filename)
            );
        }
    }

    #[test]
    fn invalid_sd_bios_is_reported_and_can_be_bypassed() {
        let fixture = Fixture::new();
        let sd = fixture.0.join("sd");
        let built_in = fixture.0.join("built-in");
        let filename = "gba.bios.bin";
        let path = sd.join(filename);
        std::fs::write(&path, []).unwrap();
        assert_eq!(file_status(&path, 16384), "Wrong size");
        // Keep the loader's existing wrong-size error when SD BIOS is enabled.
        assert_eq!(resolve_path(&sd, &built_in, filename, true), path);
        assert_eq!(
            resolve_path(&sd, &built_in, filename, false),
            built_in.join(filename)
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(file_status(&path, 16384), "Not found");
        assert_eq!(
            resolve_path(&sd, &built_in, filename, true),
            built_in.join(filename)
        );
        assert_eq!(file_status(Path::new("invalid\0bios"), 16384), "Read error");
    }

    #[test]
    fn core_preferences_are_independent_and_gba_detection_tracks_boot_mode() {
        assert!(Bios::Dmg.use_sd_bios());
        assert!(Bios::Cgb.use_sd_bios());
        assert!(Bios::Gba.use_sd_bios());
        keys::GB_USE_SD_BIOS.set(&false);
        assert_eq!(Bios::Dmg.path(), Path::new("/system/gameboy.bios-dmg.bin"));
        assert_eq!(Bios::Cgb.path(), Path::new("/system/gameboy.bios-cgb.bin"));
        assert!(Bios::Gba.use_sd_bios());

        keys::GBA_USE_SD_BIOS.set(&false);
        assert_eq!(Bios::Gba.path(), Path::new("/system/gba.bios.bin"));
        keys::GBA_SKIP_BOOT_ANIM.set(&true);
        assert_eq!(Bios::Gba.filename(), "gba.bios-fast.bin");
        assert_eq!(Bios::Gba.path(), Path::new("/system/gba.bios-fast.bin"));

        keys::GB_USE_SD_BIOS.set(&true);
        keys::GBA_USE_SD_BIOS.set(&true);
        keys::GBA_SKIP_BOOT_ANIM.set(&false);
    }
}
