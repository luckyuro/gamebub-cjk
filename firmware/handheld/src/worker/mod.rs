//! Worker threads to do background blocking work.

use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};

use crate::bitstream::CurrentBitstream;
use crate::device::drivers::fpga;
use crate::device::Device;
use crate::device::DisplayMode;
use crate::fwinfo::FirmwareVersion;
use crate::input::InputManager;
use crate::rom_list::{read_rom_list_page, RomListPageRequest};
use crate::{bitstream, kvs, ui};

#[derive(Debug)]
pub enum Message {
    /// An interrupt request from the FPGA
    FpgaIrq(u32),
    /// The headphone state has changed
    HeadphoneState(bool),
    /// The handheld has docked
    DockBegin {
        serial: u32,
        #[allow(unused)]
        hardware: u32,
        firmware: u32,
    },
    /// The handheld has undocked
    DockEnd,

    /// Run a cartridge
    RunCartridge,
    /// Persist emulated cartridge save
    SaveGame,
    /// Run a ROM file
    RunRomFile(PathBuf),
    /// Load ROM select entries
    ListRoms {
        path: PathBuf,
        page: RomListPageRequest,
    },
    /// Load Boot / Utility bitstream
    EnsureBootBitstream,
    /// The idle timer has expired
    IdleTimerExpired,
}

/// Send a message to the worker threads.
pub fn send(message: Message) {
    match SENDER.get() {
        Some(sender) => sender.send(message).unwrap(),
        None => log::error!("Dropping worker message {:?}", message),
    }
}

/// Start the worker threadpool. Called once during system init. Panics if called twice.
pub fn start() {
    let (sender, receiver) = mpsc::channel::<Message>();
    SENDER.set(sender).expect("Worker already initialized");

    // TODO: look into reducing stack usage
    std::thread::Builder::new()
        .name("Worker".to_string())
        .stack_size(16 * 1024)
        .spawn(move || {
            while let Ok(message) = receiver.recv() {
                log::debug!("Dispatch {:?}", message);
                dispatch(message);
            }
        })
        .unwrap();
}

static SENDER: OnceLock<mpsc::Sender<Message>> = OnceLock::new();

fn dispatch(message: Message) {
    match message {
        Message::FpgaIrq(irq_mask) => {
            if (irq_mask & fpga::Irq::ModuleVblank.as_flag()) != 0 {
                // Module vblank
                if let Some(bitstream) = crate::bitstream::current().get() {
                    bitstream.on_vblank_irq();
                }
            }
        }
        Message::HeadphoneState(has_headphones) => {
            log::info!("Headphone detection: {}", has_headphones);
            let mut device = Device::lock();
            device.dac.set_headphones_enabled(has_headphones).unwrap();
            device.dac.set_speakers_enabled(!has_headphones).unwrap();
        }
        Message::RunCartridge => {
            let cart_type = {
                let mut device = Device::lock();
                device.get_cart_switch()
            };
            log::info!("Cart switch: {}", cart_type);
            if cart_type {
                bitstream::current().ensure_gameboy().unwrap();
            } else {
                bitstream::current().ensure_gba().unwrap();
            };

            // Enable cartridge power after the bitstream is loaded.
            {
                let mut device = Device::lock();
                device.set_cart_power(true);
            }

            match bitstream::current().deref_mut() {
                CurrentBitstream::None => unreachable!(),
                CurrentBitstream::Gameboy(x) => x.set_physical_cartridge().unwrap(),
                CurrentBitstream::Gba(x) => x.set_physical_cartridge().unwrap(),
            }

            ui::send(ui::Message::EnterGame);
        }
        Message::SaveGame => {
            // TODO handle error more gracefully
            match bitstream::current().deref_mut() {
                CurrentBitstream::None => {}
                CurrentBitstream::Gameboy(x) => x.persist_ram().unwrap(),
                CurrentBitstream::Gba(x) => x.persist_save().unwrap(),
            }
            ui::send(ui::Message::GameSaved);
        }
        Message::RunRomFile(path) => {
            match path.extension().and_then(|e| e.to_str()) {
                Some("gbc") | Some("gb") => bitstream::current().ensure_gameboy().unwrap(),
                Some("gba") => bitstream::current().ensure_gba().unwrap(),
                _ => {
                    ui::send(ui::Message::RomSelectError(
                        "unsupported ROM file type".into(),
                    ));
                    return;
                }
            }

            let result: Result<(), String> = match bitstream::current().deref_mut() {
                CurrentBitstream::None => Err("no bitstream".into()),
                CurrentBitstream::Gameboy(x) => x
                    .set_emulated_cartridge(path.as_path())
                    .map_err(|e| e.to_string()),
                CurrentBitstream::Gba(x) => x
                    .set_emulated_cartridge(path.as_path())
                    .map_err(|e| e.to_string()),
            };
            match result {
                Ok(()) => ui::send(ui::Message::EnterGame),
                Err(err) => {
                    bitstream::current().ensure_boot().unwrap();
                    ui::send(ui::Message::RomSelectError(err))
                }
            }
        }
        Message::ListRoms { path, page } => match read_rom_list_page(&path, page) {
            Ok(page) => {
                log::info!(
                    "Listed {} ROM entries (previous={}, next={})",
                    page.entries.len(),
                    page.has_previous,
                    page.has_next
                );
                ui::send(ui::Message::RomSelectFiles(page));
            }
            Err(e) => {
                log::warn!("Error listing directory: {:?}", e);
                ui::send(ui::Message::RomSelectError(format!(
                    "Error listing directory:\n{}",
                    e,
                )));
            }
        },
        Message::DockBegin {
            serial, firmware, ..
        } => {
            ui::send(ui::Message::DockBegin {
                serial: format!("{serial:08X}"),
                firmware: format!("{}", FirmwareVersion::from(firmware)),
            });

            InputManager::lock().remove_all_gamepads();
            let mut device = Device::lock();
            device.docked = true;
            device.change_display_mode(DisplayMode::External).unwrap();
        }
        Message::DockEnd => {
            ui::send(ui::Message::DockEnd);
            InputManager::lock().remove_all_gamepads();
            let mut device = Device::lock();
            device.docked = false;
            device.change_display_mode(DisplayMode::Internal).unwrap();
        }
        Message::EnsureBootBitstream => {
            bitstream::current().ensure_boot().unwrap();
        }
        Message::IdleTimerExpired => {
            // If the idle timer expires during setup, just power off.
            let setup_stage = kvs::keys::SETUP_STAGE.get().unwrap_or_default();
            if setup_stage == 0 {
                log::warn!("Idle during setup, powering off.");
                Device::lock().power_off();
            }
            // TODO: Dim the screen temporarily.
        }
        #[allow(unreachable_patterns)]
        _ => {
            log::warn!("Unhandled message: {:?}", message);
        }
    }
}
