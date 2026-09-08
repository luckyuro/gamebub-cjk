use std::{num::NonZeroU32, sync::Arc};

use anyhow::Context as _;

use esp_idf_svc::{
    hal::{
        gpio::{InputMode, InterruptType, Pin, PinDriver},
        task::notification::{Notification, Notifier},
    },
    sys::EspError,
};

use crate::worker;
use crate::{device::drivers::fpga, ui};

use super::Device;

const FLAG_MCU_IRQ: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(1) };
const FLAG_HOME: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(2) };
const FLAG_POWER: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(4) };
const FLAG_VOL_UP: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(8) };
const FLAG_VOL_DOWN: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(16) };
const FLAG_VBUS_PGOOD: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(32) };
const FLAG_BUTTONS: u32 =
    FLAG_HOME.get() | FLAG_POWER.get() | FLAG_VOL_UP.get() | FLAG_VOL_DOWN.get();

fn setup_gpio_interrupt(
    pin: &mut PinDriver<'_, impl Pin, impl InputMode>,
    interrupt_type: InterruptType,
    notifier: Arc<Notifier>,
    flags: NonZeroU32,
) -> Result<(), EspError> {
    // SAFETY: only ISR-safe FreeRTOS functions will be called (task notify).
    unsafe {
        pin.subscribe(move || {
            notifier.notify_and_yield(flags);
        })?;
    }
    pin.set_interrupt_type(interrupt_type)?;
    pin.enable_interrupt()?;
    Ok(())
}

impl Device<'_> {
    /// Setup interrupts on the Device interrupt sources:
    ///
    /// * Volume up, volume down, home, and power buttons
    /// * Shared MCU_IRQ line
    pub(super) fn setup_interrupts() -> anyhow::Result<()> {
        // Setup interrupt handler thread.
        std::thread::Builder::new()
            .name("Interrupt".to_string())
            .stack_size(4 * 1024)
            .spawn(|| {
                let notification = Notification::new();

                {
                    let device = &mut Device::get().lock().unwrap();
                    macro_rules! setup {
                        ($pin:expr, $kind:expr, $flag:expr, $name:literal) => {
                            if let Err(error) = setup_gpio_interrupt(
                                $pin,
                                $kind,
                                notification.notifier(),
                                $flag,
                            ) {
                                log::error!("Failed to configure {} interrupt: {error}", $name);
                            }
                        };
                    }
                    setup!(
                        &mut device.button_home,
                        InterruptType::AnyEdge,
                        FLAG_HOME,
                        "home button"
                    );
                    setup!(
                        &mut device.button_power,
                        InterruptType::AnyEdge,
                        FLAG_POWER,
                        "power button"
                    );
                    setup!(
                        &mut device.button_vol_up,
                        InterruptType::AnyEdge,
                        FLAG_VOL_UP,
                        "volume-up button"
                    );
                    setup!(
                        &mut device.button_vol_down,
                        InterruptType::AnyEdge,
                        FLAG_VOL_DOWN,
                        "volume-down button"
                    );
                    setup!(
                        &mut device.pin_irq,
                        InterruptType::LowLevel,
                        FLAG_MCU_IRQ,
                        "shared MCU IRQ"
                    );
                    setup!(
                        &mut device.pin_vbus_pgood,
                        InterruptType::AnyEdge,
                        FLAG_VBUS_PGOOD,
                        "VBUS power-good"
                    );
                }

                #[allow(unused)]
                let mut prev_hdmi_detected: Option<bool> = None;
                #[allow(unused)]
                let mut prev_vbus_pgood: Option<bool> = None;

                loop {
                    let flags = match notification.wait(esp_idf_svc::hal::delay::BLOCK) {
                        Some(flags) => flags.get(),
                        _ => continue,
                    };

                    let mut device = Device::get().lock().unwrap();

                    if (flags & FLAG_HOME.get()) != 0 {
                        let _ = device.button_home.enable_interrupt();
                    }
                    if (flags & FLAG_POWER.get()) != 0 {
                        let _ = device.button_power.enable_interrupt();
                    }
                    if (flags & FLAG_VOL_UP.get()) != 0 {
                        let _ = device.button_vol_up.enable_interrupt();
                    }
                    if (flags & FLAG_VOL_DOWN.get()) != 0 {
                        let _ = device.button_vol_down.enable_interrupt();
                    }
                    if (flags & FLAG_VBUS_PGOOD.get()) != 0 {
                        let _ = device.pin_vbus_pgood.enable_interrupt();
                    }
                    let mut poll_buttons = (flags & FLAG_BUTTONS) != 0;

                    // Rev 1 and 2, must read I/O expander to clear IRQ.
                    #[cfg(feature = "has_io_expander")]
                    #[allow(unused)]
                    let io_expander = match device.io_expander.get_pins() {
                        Ok(pins) => Some(pins),
                        Err(error) => {
                            log::error!("Failed to read I/O expander interrupt state: {error}");
                            None
                        }
                    };

                    // Handle dock monitoring
                    cfg_if::cfg_if! {
                        if #[cfg(feature = "rev1")] {
                            // Docking is based on HDMI hot plug
                            if let Some(io_expander) = io_expander {
                                match device.parse_hdmi_detect(io_expander) {
                                    Ok(hdmi_detected) if prev_hdmi_detected != Some(hdmi_detected) => {
                                        prev_hdmi_detected = Some(hdmi_detected);

                                        if hdmi_detected {
                                            worker::send(worker::Message::DockBegin { serial: 0, hardware: 0, firmware: 0 });
                                        } else {
                                            worker::send(worker::Message::DockEnd);
                                        }
                                    }
                                    Ok(_) => {}
                                    Err(()) => log::error!("Failed to parse HDMI detect state"),
                                }
                            }
                        } else {
                            // On VBUS pgood falling, force undock
                            let vbus_pgood = device.get_vbus_pgood();
                            if prev_vbus_pgood != Some(vbus_pgood) {
                                prev_vbus_pgood = Some(vbus_pgood);
                                if !vbus_pgood {
                                    worker::send(worker::Message::DockEnd);
                                }
                            }
                        }
                    }

                    if (flags & FLAG_MCU_IRQ.get()) != 0 {
                        log::debug!("Interrupt: MCU_IRQ");

                        // Fuel gauge IRQs.
                        #[cfg(feature = "has_max17048")]
                        if let Ok(fuel_irq) = device.fuel_gauge.query_alerts() {
                            let _ = fuel_irq;
                        }

                        // DAC IRQs.
                        if let Ok(dac_irq) = device.dac.get_interrupt_status() {
                            if dac_irq.headset_detected {
                                match device.dac.get_headphones_detected() {
                                    Ok(has_headphones) => worker::send(
                                        worker::Message::HeadphoneState(has_headphones),
                                    ),
                                    Err(error) => {
                                        log::error!("Failed to read headphone state: {error:?}")
                                    }
                                }
                            }
                        }

                        // FPGA IRQs
                        match device.fpga.read_u32(fpga::REG_IRQ_STATUS) {
                            Ok(fpga_irq) => {
                                if fpga_irq != 0 {
                                    if let Err(error) = device
                                        .fpga
                                        .write_u32(fpga::REG_IRQ_STATUS, fpga_irq)
                                    {
                                        log::error!("Failed to acknowledge FPGA IRQ: {error}");
                                    }
                                    worker::send(worker::Message::FpgaIrq(fpga_irq));
                                }
                                if (fpga_irq & fpga::Irq::Button.as_flag()) != 0 {
                                    poll_buttons = true;
                                }
                            }
                            Err(error) => log::error!("Failed to read FPGA IRQ status: {error}"),
                        }

                        let _ = device.pin_irq.enable_interrupt();
                    }

                    if poll_buttons {
                        match device.get_input_state() {
                            Ok(input_state) => ui::send(ui::Message::InputState(input_state)),
                            Err(()) => log::error!("Failed to read input state"),
                        }
                    }
                }
            })
            .context("failed to start interrupt handler")?;
        Ok(())
    }
}
