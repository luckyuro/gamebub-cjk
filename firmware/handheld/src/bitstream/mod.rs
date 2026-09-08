use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;
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

pub static SCRATCH: ScratchBuffer<{ 16 * 1024 }> = ScratchBuffer::new();

/// Driver for a specific bitstream.
pub trait Bitstream {
    /// Called when a vblank IRQ occurs.
    fn on_vblank_irq(&mut self);
}

pub fn program_fpga(path: &Path) -> Result<(), String> {
    log::info!("Loading bitstream {}", path.display());
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

        let file = File::open(path)
            .map_err(|e| format!("failed to open bitstream {}: {e}", path.display()))?;
        let mut bitstream: Box<dyn Read> = if path.extension() == Some(OsStr::new("hs")) {
            Box::new(heatshrink_decompress_stream(file))
        } else if path.extension() == Some(OsStr::new("bit")) {
            Box::new(BufReader::with_capacity(512, file))
        } else {
            return Err("Unsupported bitstream extension".into());
        };
        let mut scratch = SCRATCH
            .take()
            .ok_or_else(|| "FPGA scratch buffer is already in use".to_string())?;

        device
            .fpga
            .program(&mut bitstream, &mut scratch)
            .map_err(|error| format!("failed to program bitstream {}: {error}", path.display()))?;
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

pub fn initial_program_boot(device: &mut Device) -> anyhow::Result<()> {
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

pub fn program_boot() -> Result<(), String> {
    program_fpga(&crate::util::get_system_file_path("boot.bit.hs"))
}
