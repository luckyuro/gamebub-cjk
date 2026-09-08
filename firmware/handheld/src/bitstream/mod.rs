use std::fs::File;
use std::io::Read;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use crate::bitstream::util::scratch_buffer::ScratchBuffer;
use crate::device::DisplayMode;
use crate::device::{drivers::fpga, Device};
use crate::led;
use crate::ui;

pub mod boot;
pub mod gameboy;
pub mod gba;

mod util;

static SCRATCH: ScratchBuffer<{ 16 * 1024 }> = ScratchBuffer::new();

/// Driver for a specific bitstream.
pub trait Bitstream {
    /// Get the path for the bitstream.
    fn get_bitstream_path(&self) -> &'static str;

    /// Do final initialization after programming the bitstream.
    fn on_after_program(&mut self) -> Result<(), String>;

    /// Set whether the inner design is paused.
    fn set_paused(&mut self, paused: bool) -> Result<(), fpga::Error>;

    /// Reset the inner design, leaving it paused.
    fn reset(&mut self) -> Result<(), fpga::Error>;

    /// Called when a vblank IRQ occurs.
    fn on_vblank_irq(&mut self);
}

/// The current global bitstream, behind a lock.
static CURRENT: Mutex<CurrentBitstream> = Mutex::new(CurrentBitstream::None);

/// Lock and return the current bitstream.
pub fn current() -> MutexGuard<'static, CurrentBitstream> {
    CURRENT.lock().unwrap()
}

fn program_fpga(path: &str) -> Result<(), String> {
    log::info!("Loading bitstream {}", path);
    led::LedController::set_behavior(led::LedBehavior::LOADING);
    let mut device = Device::lock();
    let display_mode = device.get_display_mode();

    let result = (|| {
        if let DisplayMode::Internal = display_mode {
            // Avoid LCD artifacts during FPGA reprogram.
            device
                .set_lcd_enabled(false)
                .map_err(|error| format!("failed to put LCD to sleep: {error:#}"))?;
            // For some reason, we need to sleep for a short amount of time here
            // (before doing FPGA program), otherwise the LCD won't properly sleep.
            // 2 ms is sometimes sufficient, 5 ms is always sufficient, 10 ms seems to always work.
            std::thread::sleep(Duration::from_millis(10));
        }

        let file = crate::util::open_system_file(path)
            .map_err(|error| format!("failed to open bitstream {path}: {error}"))?;
        let mut bitstream = heatshrink_decompress_stream(file);
        let mut scratch = SCRATCH
            .take()
            .ok_or_else(|| "FPGA scratch buffer is already in use".to_string())?;

        device
            .fpga
            .program(&mut bitstream, &mut scratch)
            .map_err(|error| format!("failed to program bitstream {path}: {error}"))?;
        device
            .fpga
            .set_display_mode(display_mode)
            .map_err(|error| format!("failed to restore display mode: {error}"))?;
        device
            .fpga
            .enable_interrupt(fpga::Irq::Button)
            .map_err(|error| format!("failed to enable button interrupt: {error}"))?;
        let input_state = device
            .get_input_state()
            .map_err(|()| "failed to read input state".to_string())?;
        ui::send(ui::Message::InputState(input_state));
        ui::send(ui::Message::Redraw);
        Ok(())
    })();

    led::LedController::set_behavior(led::LedBehavior::OFF);

    let lcd_result = if let DisplayMode::Internal = display_mode {
        device
            .set_lcd_enabled(true)
            .map_err(|error| format!("failed to wake LCD: {error:#}"))
    } else {
        Ok(())
    };

    match (result, lcd_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(lcd_error)) => Err(format!("{error}; {lcd_error}")),
    }
}

pub fn program_boot(device: &mut Device) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let file = crate::util::open_system_file("boot.bit.hs").context("Failed to read bitstream")?;
    let mut bitstream = heatshrink_decompress_stream(file);

    let mut scratch = SCRATCH
        .take()
        .context("FPGA scratch buffer is already in use")?;
    device
        .fpga
        .program(&mut bitstream, &mut scratch)
        .context("Failed to program FPGA")
}

fn heatshrink_decompress_stream(file: File) -> impl Read {
    // Heatshrink decoder parameters: W=9, L=6 (chosen empirically)
    type HeatshrinkDecoder = heatshrink::decoder::HeatshrinkDecoder<9, 6, 512, 512>;
    let reader = embedded_io_adapters::std::FromStd::new(file);
    let decoder = heatshrink::io::DecoderReader::<_, HeatshrinkDecoder>::new(reader);
    embedded_io_adapters::std::ToStd::new(decoder)
}

pub enum CurrentBitstream {
    None,
    Gameboy(gameboy::Gameboy),
    Gba(gba::Gba),
    // TODO: add "Boot" variant to distinguish between actually None and Boot.
}

impl CurrentBitstream {
    pub fn get(&mut self) -> Option<&mut dyn Bitstream> {
        match self {
            CurrentBitstream::None => None,
            CurrentBitstream::Gameboy(x) => Some(x),
            CurrentBitstream::Gba(x) => Some(x),
        }
    }

    fn set(&mut self, mut new: CurrentBitstream) -> Result<(), String> {
        if let Some(bitstream) = new.get() {
            let result = program_fpga(bitstream.get_bitstream_path())
                .and_then(|()| bitstream.on_after_program());
            if let Err(error) = result {
                // A failed configuration can leave the FPGA partially programmed. Restore the
                // boot design here because `None` also represents an already-loaded boot design.
                return match program_fpga("boot.bit.hs") {
                    Ok(()) => {
                        *self = CurrentBitstream::None;
                        Err(error)
                    }
                    Err(recovery_error) => Err(format!(
                        "{error}; additionally failed to restore boot bitstream: {recovery_error}"
                    )),
                };
            }
        }
        *self = new;
        Ok(())
    }

    /// Ensure the boot is loaded.
    pub fn ensure_boot(&mut self) -> Result<(), String> {
        match self {
            CurrentBitstream::None => Ok(()),
            _ => {
                program_fpga("boot.bit.hs")?;
                *self = CurrentBitstream::None;
                Ok(())
            }
        }
    }

    /// Ensure the gameboy bitstream is loaded.
    pub fn ensure_gameboy(&mut self) -> Result<(), String> {
        match self {
            CurrentBitstream::Gameboy(_) => Ok(()),
            _ => {
                let bitstream = gameboy::Gameboy::new();
                self.set(CurrentBitstream::Gameboy(bitstream))
            }
        }
    }

    /// Ensure the GBA bitstream is loaded.
    pub fn ensure_gba(&mut self) -> Result<(), String> {
        match self {
            CurrentBitstream::Gba(_) => Ok(()),
            _ => {
                let bitstream = gba::Gba::new();
                self.set(CurrentBitstream::Gba(bitstream))
            }
        }
    }
}
