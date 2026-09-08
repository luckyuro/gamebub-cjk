//! Worker threads to do background blocking work.

use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};

use crate::core::CoreManager;
use crate::device::drivers::fpga;
use crate::device::Device;
use crate::device::DisplayMode;
use crate::fwinfo::FirmwareVersion;
use crate::input::InputManager;
use crate::{kvs, ui};

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
    /// The idle timer has expired
    IdleTimerExpired,

    /// Fetch the list of cores
    FetchCoreList,
    /// Start running a core (ID)
    RunCore(String),
    /// Exit the current core (or cancel loading)
    ExitCore,
    /// A file was selected for the core (could be a directory).
    CoreFileSelected(PathBuf),
    CoreFileList {
        path: PathBuf,
        page: crate::rom_list::RomListPageRequest,
    },
    /// Core file selection was cancelled.
    CoreFileCancelled,
    /// Core focus changed
    CoreFocusChanged(bool),
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

fn dispatch(message: Message) {
    match message {
        Message::FpgaIrq(irq_mask) => {
            if (irq_mask & fpga::Irq::ModuleVblank.as_flag()) != 0 {
                // Module vblank
                if let Some(bitstream) = CoreManager::lock().current_bitstream() {
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
            let cart_type = {
                let mut device = Device::lock();
                match device.get_cart_switch() {
                    Ok(value) => value,
                    Err(error) => {
                        ui::send(ui::Message::CoreLoadError(format!(
                            "Failed to read cartridge switch: {error}"
                        )));
                        return;
                    }
                }
            };
            log::info!("Cart switch: {}", cart_type);

            let core_id = if cart_type {
                "Game-Bub.GB"
            } else {
                "Game-Bub.GBA"
            };

            CoreManager::lock().run_core(core_id, true);
        }
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
        Message::IdleTimerExpired => {
            // If the idle timer expires during setup, just power off.
            let setup_stage = kvs::keys::SETUP_STAGE.get().unwrap_or_default();
            if setup_stage == 0 {
                log::warn!("Idle during setup, powering off.");
                Device::lock().power_off();
            }
            // TODO: Dim the screen temporarily.
        }
        Message::FetchCoreList => CoreManager::lock().list_cores(),
        Message::RunCore(id) => CoreManager::lock().run_core(&id, false),
        Message::ExitCore => CoreManager::lock().exit_core(),
        Message::CoreFileList { path, page } => {
            CoreManager::lock().send_core_file_list(&path, page)
        }
        Message::CoreFileSelected(file) => CoreManager::lock().handle_file_selected(file),
        Message::CoreFileCancelled => CoreManager::lock().cancel_file_select(),
        Message::CoreFocusChanged(focused) => CoreManager::lock().focus_changed(focused),
        #[allow(unreachable_patterns)]
        _ => {
            log::warn!("Unhandled message: {:?}", message);
        }
    }
}
