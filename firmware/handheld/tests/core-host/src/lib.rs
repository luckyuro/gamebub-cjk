#![allow(dead_code)]
extern crate self as esp_idf_svc;
pub mod hal {
    pub mod units {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub struct Hertz(pub u32);
    }
}
#[path = "../../../src/rom_list.rs"]
mod rom_list;
mod util {
    pub mod background_io {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/util/background_io.rs"
        ));
    }
}
mod ui {
    use std::{cell::RefCell, path::PathBuf};
    pub enum Message {
        CoreList(Vec<crate::core::CoreListEntry>),
        CoreFileSelectBegin { label: String, path: PathBuf },
        CoreFileSelectList(crate::rom_list::RomListPage),
        CoreFileSelectError(String),
        CoreLoadError(String),
        CoreLoadProgress(f32),
        EnterGame,
    }
    thread_local! { pub static QUEUE: RefCell<Vec<Message>> = const { RefCell::new(Vec::new()) }; }
    pub fn send(message: Message) {
        QUEUE.with(|q| q.borrow_mut().push(message));
    }
}
mod device {
    use std::sync::{LazyLock, Mutex, MutexGuard};
    pub mod drivers {
        pub mod fpga {
            use crate::hal::units::Hertz;
            pub const MAX_SPI_READ_CLOCK: Hertz = Hertz(20_000_000);
            pub const REG_CMD_HOST_BASE: u32 = 0x100;
            pub const REG_CTRL_CMD_HOST: u32 = 0x200;
            pub const REG_CTRL_FOCUS: u32 = 0x204;
            pub const REG_CTRL_VIBRATE: u32 = 0x208;
            #[derive(Clone, Copy)]
            pub enum FpgaSpiWordSize {
                Bits8,
                Bits16,
                Bits32,
            }
            pub struct SpiCommand;
            impl SpiCommand {
                pub fn new(_: FpgaSpiWordSize) -> Self {
                    Self
                }
            }
            #[derive(Default)]
            pub struct Fpga {
                pub fail_read: bool,
                pub fail_write: bool,
                pub fail_spi_read: bool,
                pub fail_spi_write: bool,
                pub transferred: Vec<u8>,
            }
            impl Fpga {
                pub fn write_u32(&mut self, _: u32, _: u32) -> Result<(), &'static str> {
                    if self.fail_write {
                        Err("injected SPI write failure")
                    } else {
                        Ok(())
                    }
                }
                pub fn read_u32(&mut self, address: u32) -> Result<u32, &'static str> {
                    if self.fail_read {
                        Err("injected SPI read failure")
                    } else if address == REG_CTRL_CMD_HOST {
                        Ok(2)
                    } else {
                        Ok(32)
                    }
                }
                pub fn spi_write(
                    &mut self,
                    _: Option<Hertz>,
                    _: SpiCommand,
                    _: u32,
                    data: &[u8],
                ) -> Result<(), &'static str> {
                    if self.fail_spi_write {
                        return Err("injected SPI data write failure");
                    }
                    self.transferred.extend_from_slice(data);
                    Ok(())
                }
                pub fn spi_read(
                    &mut self,
                    _: Option<Hertz>,
                    _: SpiCommand,
                    _: u32,
                    data: &mut [u8],
                ) -> Result<(), &'static str> {
                    if self.fail_spi_read {
                        return Err("injected SPI data read failure");
                    }
                    data.fill(0x5a);
                    Ok(())
                }
            }
        }
    }
    #[derive(Default)]
    pub struct Dac;
    impl Dac {
        pub fn set_power_down(&mut self, _: u8, _: bool) -> Result<(), &'static str> {
            Ok(())
        }
    }
    #[derive(Default)]
    pub struct Device {
        pub fpga: drivers::fpga::Fpga,
        pub dac: Dac,
    }
    static DEVICE: LazyLock<Mutex<Device>> = LazyLock::new(|| Mutex::new(Device::default()));
    impl Device {
        pub fn lock() -> MutexGuard<'static, Self> {
            DEVICE.lock().unwrap_or_else(|e| e.into_inner())
        }
        pub fn set_cart_power(&mut self, _: bool) -> Result<(), &'static str> {
            Ok(())
        }
    }
}
mod bitstream {
    use std::{
        fs::File,
        path::{Path, PathBuf},
    };
    mod scratch_buffer {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/bitstream/util/scratch_buffer.rs"
        ));
    }
    pub static SCRATCH: scratch_buffer::ScratchBuffer<1024> = scratch_buffer::ScratchBuffer::new();
    pub trait Bitstream {}
    pub fn program_boot() -> Result<(), String> {
        Ok(())
    }
    pub fn program_fpga(_: &Path) -> Result<(), String> {
        Ok(())
    }
    pub struct MockCore;
    impl MockCore {
        pub fn new() -> Self {
            Self
        }
        pub fn get_core_info() -> crate::core::CoreInfo {
            panic!("unexpected core discovery")
        }
    }
    impl Bitstream for MockCore {}
    impl crate::core::CoreHandler for MockCore {
        fn on_after_program(&mut self) {}
        fn get_file_path_override(&mut self, _: u16) -> Option<PathBuf> {
            None
        }
        fn on_before_file_load(&mut self, _: u16, _: &mut File) -> Result<(), String> {
            Ok(())
        }
        fn on_before_run(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn on_focus_changed(&mut self, _: bool) {}
        fn get_file_size(&mut self, _: u16) -> Option<u32> {
            None
        }
        fn on_after_file_save(&mut self, _: u16, _: &mut File) -> Result<(), String> {
            Ok(())
        }
        fn as_legacy_bitstream(&mut self) -> &mut dyn Bitstream {
            self
        }
    }
    pub mod gameboy {
        pub type Gameboy = super::MockCore;
    }
    pub mod gba {
        pub type Gba = super::MockCore;
    }
}
mod core {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/core/mod.rs"
    ));
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SERIAL: Mutex<()> = Mutex::new(());
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        struct Fixture {
            manager: CoreManager,
            dir: PathBuf,
            _guard: MutexGuard<'static, ()>,
        }
        impl Fixture {
            fn new(stage: Stage) -> Self {
                let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
                *Device::lock() = Default::default();
                ui::QUEUE.with(|q| q.borrow_mut().clear());
                let dir = std::env::temp_dir().join(format!(
                    "gamebub-core-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir(&dir).unwrap();
                let file = CoreFile {
                    id: 0,
                    label: "ROM".try_into().unwrap(),
                    filename: None,
                    extensions: [".gba".try_into().unwrap()].into_iter().collect(),
                    optional: true,
                    read_only: true,
                    user_selected: true,
                    dependent_on_0: false,
                    initialize: false,
                    address: 0x3000_0000,
                    max_size: 32 * 1024 * 1024,
                    exact_size: 0,
                    max_transfer_speed: 20_000,
                    transfer_word_size: fpga::FpgaSpiWordSize::Bits32,
                };
                let mut manager = CoreManager::new();
                manager.stage = stage;
                manager.core_info = Some(CoreInfo {
                    id: "Test".try_into().unwrap(),
                    name: "Test".try_into().unwrap(),
                    author: "Test".try_into().unwrap(),
                    files: [file].into_iter().collect(),
                    bitstream: dir.join("core.bit"),
                });
                manager.core_settings = Some(Default::default());
                manager.selected_files = vec![Some(dir.join("game.gba"))];
                Self {
                    manager,
                    dir,
                    _guard: guard,
                }
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.dir).unwrap();
            }
        }
        #[test]
        fn late_cancel_preserves_running_core_and_save_paths() {
            let mut f = Fixture::new(Stage::Running);
            f.manager.cancel_file_select();
            assert_eq!(f.manager.stage, Stage::Running);
            assert!(f.manager.core_info.is_some());
            assert!(f.manager.selected_files[0].is_some());
        }
        #[test]
        fn cancellation_releases_all_selection_state() {
            let mut f = Fixture::new(Stage::LoadSelectFile(0));
            f.manager.cancel_file_select();
            assert_eq!(f.manager.stage, Stage::Idle);
            assert!(f.manager.core_info.is_none());
            assert!(f.manager.core_settings.is_none());
            assert!(f.manager.selected_files.is_empty());
        }
        #[test]
        fn missing_selection_returns_visible_error() {
            let mut f = Fixture::new(Stage::LoadSelectFile(0));
            f.manager.handle_file_selected(f.dir.join("deleted.gba"));
            assert_eq!(f.manager.stage, Stage::LoadSelectFile(0));
            ui::QUEUE.with(|q| {
                assert!(q
                    .borrow()
                    .iter()
                    .any(|m| matches!(m, ui::Message::CoreFileSelectError(_))))
            });
        }
        #[test]
        fn queued_selection_after_cancel_is_ignored() {
            let mut f = Fixture::new(Stage::LoadSelectFile(0));
            let path = f.dir.join("game.gba");
            std::fs::write(&path, b"ROM").unwrap();
            f.manager.cancel_file_select();
            f.manager.handle_file_selected(path);
            assert_eq!(f.manager.stage, Stage::Idle);
        }
        #[test]
        fn duplicate_core_start_is_ignored() {
            let mut f = Fixture::new(Stage::LoadSelectFile(0));
            f.manager.run_core("Game-Bub.GBA", false);
            assert_eq!(f.manager.stage, Stage::LoadSelectFile(0));
        }
        #[test]
        fn selected_rom_removed_before_open_is_not_optional() {
            let mut f = Fixture::new(Stage::LoadBitstream);
            assert!(matches!(
                f.manager.load_files(),
                Err(CoreError::CannotOpenFile(_))
            ));
        }
        #[test]
        fn command_read_failure_is_returned_without_panic() {
            let f = Fixture::new(Stage::LoadBitstream);
            Device::lock().fpga.fail_read = true;
            assert!(f
                .manager
                .run_core_command(&[command::GET_STATUS], &mut [0], NOTIFY_TIMEOUT)
                .is_err());
        }
        #[test]
        fn command_write_failure_is_not_reported_as_success() {
            let f = Fixture::new(Stage::LoadBitstream);
            Device::lock().fpga.fail_write = true;
            assert!(f
                .manager
                .run_core_command(&[command::GET_STATUS], &mut [], NOTIFY_TIMEOUT)
                .is_err());
        }
        #[test]
        fn file_data_reaches_spi_exactly_once() {
            let mut f = Fixture::new(Stage::LoadBitstream);
            let bytes: Vec<u8> = (0..5003).map(|i| (i % 251) as u8).collect();
            std::fs::write(f.dir.join("game.gba"), &bytes).unwrap();
            f.manager.load_files().unwrap();
            assert_eq!(Device::lock().fpga.transferred, bytes);
            assert!(crate::bitstream::SCRATCH.take().is_some());
        }
        #[test]
        fn data_write_failure_stops_load_and_releases_scratch() {
            let mut f = Fixture::new(Stage::LoadBitstream);
            std::fs::write(f.dir.join("game.gba"), vec![0x5a; 5003]).unwrap();
            Device::lock().fpga.fail_spi_write = true;
            assert!(matches!(
                f.manager.load_files(),
                Err(CoreError::FailedLoadFile(_, _))
            ));
            assert!(Device::lock().fpga.transferred.is_empty());
            assert!(crate::bitstream::SCRATCH.take().is_some());
        }
        #[test]
        fn save_memory_initialization_propagates_spi_failure() {
            let f = Fixture::new(Stage::LoadBitstream);
            Device::lock().fpga.fail_spi_write = true;
            let info = &f.manager.core_info.as_ref().unwrap().files[0];
            assert!(f.manager.clear_file_slot(info, &mut [0; 32]).is_err());
        }
        #[test]
        fn stream_callback_error_stops_before_next_chunk() {
            let f = Fixture::new(Stage::LoadBitstream);
            let path = f.dir.join("data.bin");
            std::fs::write(&path, vec![1; 4096]).unwrap();
            let mut calls = 0;
            let result = crate::util::background_io::iter_chunks(
                File::open(&path).unwrap(),
                &mut [0; 512],
                |_| {
                    calls += 1;
                    Err::<(), _>(std::io::Error::other("injected transfer failure"))
                },
            );
            assert!(result.is_err());
            assert_eq!(calls, 1);
            let result = crate::util::background_io::iter_chunks::<std::io::Error>(
                File::open(&path).unwrap(),
                &mut [],
                |_| panic!("empty buffer reached consumer"),
            );
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);
        }
        #[test]
        fn listing_uses_core_filter_and_reports_failure_once() {
            let f = Fixture::new(Stage::LoadSelectFile(0));
            std::fs::write(f.dir.join("A.GBA"), b"").unwrap();
            std::fs::write(f.dir.join("B.gb"), b"").unwrap();
            f.manager
                .send_core_file_list(&f.dir, RomListPageRequest::First);
            ui::QUEUE.with(|q| match q.borrow_mut().pop().unwrap() {
                ui::Message::CoreFileSelectList(page) => assert_eq!(page.entries[0].name, "A.GBA"),
                _ => panic!("missing page"),
            });
            f.manager
                .send_core_file_list(&f.dir.join("missing"), RomListPageRequest::First);
            ui::QUEUE.with(|q| {
                let q = q.borrow();
                assert_eq!(q.len(), 1);
                assert!(matches!(q[0], ui::Message::CoreFileSelectError(_)));
            });
        }
    }
}
