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
        Some(sender) => {
            if let Err(mpsc::SendError(message)) = sender.send(message) {
                log::error!(
                    "Dropping worker message because the worker stopped: {:?}",
                    message
                );
            }
        }
        None => log::error!("Dropping worker message {:?}", message),
    }
}

/// Start the worker threadpool. Called once during system init.
pub fn start() -> anyhow::Result<()> {
    let (sender, receiver) = mpsc::channel::<Message>();
    SENDER
        .set(sender)
        .map_err(|_| anyhow::anyhow!("worker already initialized"))?;

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
        .map_err(|error| anyhow::anyhow!("failed to start worker thread: {error}"))?;
    Ok(())
}

static SENDER: OnceLock<mpsc::Sender<Message>> = OnceLock::new();

fn recover_to_boot(context: &str, error: impl std::fmt::Display) -> String {
    let mut message = format!("{context}: {error}");
    log::error!("{message}");
    if let Err(recovery_error) = bitstream::current().ensure_boot() {
        log::error!("Failed to restore boot bitstream: {recovery_error}");
        message.push_str("\nFailed to restore the boot screen: ");
        message.push_str(&recovery_error);
    }
    message
}

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
            if let Err(error) = device.dac.set_headphones_enabled(has_headphones) {
                log::error!("Failed to update headphone output: {error:?}");
            }
            if let Err(error) = device.dac.set_speakers_enabled(!has_headphones) {
                log::error!("Failed to update speaker output: {error:?}");
            }
        }
        Message::RunCartridge => {
            let result = (|| -> Result<(), String> {
                let cart_type = Device::lock()
                    .get_cart_switch()
                    .map_err(|error| format!("failed to read cartridge switch: {error}"))?;
                log::info!("Cart switch: {}", cart_type);

                let mut current = bitstream::current();
                if cart_type {
                    current.ensure_gameboy()?;
                } else {
                    current.ensure_gba()?;
                }

                // Enable cartridge power after the bitstream is loaded.
                Device::lock()
                    .set_cart_power(true)
                    .map_err(|error| format!("failed to enable cartridge power: {error:#}"))?;

                match current.deref_mut() {
                    CurrentBitstream::None => Err("no cartridge bitstream is loaded".into()),
                    CurrentBitstream::Gameboy(gameboy) => gameboy
                        .set_physical_cartridge()
                        .map_err(|error| error.to_string()),
                    CurrentBitstream::Gba(gba) => gba
                        .set_physical_cartridge()
                        .map_err(|error| error.to_string()),
                }
            })();

            match result {
                Ok(()) => ui::send(ui::Message::EnterGame),
                Err(error) => {
                    if let Err(power_error) = Device::lock().set_cart_power(false) {
                        log::error!("Failed to disable cartridge power: {power_error:#}");
                    }
                    let message = recover_to_boot("failed to start cartridge", error);
                    ui::send(ui::Message::FatalError(message));
                }
            }
        }
        Message::SaveGame => {
            match bitstream::current().deref_mut() {
                CurrentBitstream::None => {}
                CurrentBitstream::Gameboy(gameboy) => {
                    if let Err(error) = gameboy.persist_ram() {
                        log::error!("Failed to save Game Boy cartridge RAM: {error}");
                    }
                }
                CurrentBitstream::Gba(gba) => {
                    if let Err(error) = gba.persist_save() {
                        log::error!("Failed to save GBA cartridge RAM: {error}");
                    }
                }
            }
            ui::send(ui::Message::GameSaved);
        }
        Message::RunRomFile(path) => {
            crate::util::log_memory_checkpoint("before ROM load");
            let extension = path.extension().and_then(|extension| extension.to_str());
            let result = (|| -> Result<(), String> {
                let mut current = bitstream::current();
                match extension {
                    Some(extension)
                        if extension.eq_ignore_ascii_case("gb")
                            || extension.eq_ignore_ascii_case("gbc") =>
                    {
                        current.ensure_gameboy()?
                    }
                    Some(extension) if extension.eq_ignore_ascii_case("gba") => {
                        current.ensure_gba()?
                    }
                    _ => return Err("unsupported ROM file type".into()),
                }
                crate::util::log_memory_checkpoint("after game bitstream load");

                match current.deref_mut() {
                    CurrentBitstream::None => Err("no game bitstream is loaded".into()),
                    CurrentBitstream::Gameboy(gameboy) => gameboy
                        .set_emulated_cartridge(path.as_path())
                        .map_err(|error| error.to_string()),
                    CurrentBitstream::Gba(gba) => gba
                        .set_emulated_cartridge(path.as_path())
                        .map_err(|error| error.to_string()),
                }
            })();
            match result {
                Ok(()) => ui::send(ui::Message::EnterGame),
                Err(err) => {
                    let message = recover_to_boot("failed to load ROM", err);
                    ui::send(ui::Message::RomSelectError(message))
                }
            }
            crate::util::log_memory_checkpoint("after ROM load");
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
            if let Err(error) = device.change_display_mode(DisplayMode::External) {
                log::error!("Failed to switch to external display: {error:#}");
                ui::send(ui::Message::FatalError(format!(
                    "Failed to switch to external display: {error:#}"
                )));
            }
        }
        Message::DockEnd => {
            ui::send(ui::Message::DockEnd);
            InputManager::lock().remove_all_gamepads();
            let mut device = Device::lock();
            device.docked = false;
            if let Err(error) = device.change_display_mode(DisplayMode::Internal) {
                log::error!("Failed to switch to internal display: {error:#}");
                ui::send(ui::Message::FatalError(format!(
                    "Failed to switch to internal display: {error:#}"
                )));
            }
        }
        Message::EnsureBootBitstream => {
            if let Err(error) = bitstream::current().ensure_boot() {
                let message = format!("Failed to restore boot bitstream: {error}");
                log::error!("{message}");
                ui::send(ui::Message::FatalError(message));
            }
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
