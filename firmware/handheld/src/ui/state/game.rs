use std::{cell::RefCell, ops::DerefMut, rc::Rc, time::Duration};

use super::super::slint::Backend;
use slint::{ComponentHandle, Timer};

use crate::{
    bitstream::{self, Bitstream, CurrentBitstream},
    device::Device,
    ui::slint::ScreenId,
    worker,
};

use super::UiState;

impl UiState {
    /// Set up the "Game" screen.
    pub(super) fn setup_game(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_game_set_paused(move |paused| {
            let result = match bitstream::current().deref_mut() {
                CurrentBitstream::None => Ok(false),
                CurrentBitstream::Gameboy(x) => {
                    x.set_paused(paused).map(|()| x.needs_save_persist())
                }
                CurrentBitstream::Gba(x) => x.set_paused(paused).map(|()| x.needs_save_persist()),
            };
            let needs_persist = match result {
                Ok(needs_persist) => needs_persist,
                Err(error) => {
                    let message = format!("Failed to update game pause state: {error}");
                    log::error!("{message}");
                    crate::ui::send(crate::ui::Message::FatalError(message));
                    return;
                }
            };
            if paused && needs_persist {
                let state = state_.borrow_mut();
                let root = state.root.unwrap();
                let backend = root.global::<Backend>();
                backend.set_status_is_saving(true);
                worker::send(worker::Message::SaveGame);
            }
        });

        backend.on_game_reset(move || {
            let result = match bitstream::current().deref_mut() {
                CurrentBitstream::None => Ok(()),
                CurrentBitstream::Gameboy(x) => x.reset(),
                CurrentBitstream::Gba(x) => x.reset(),
            };
            if let Err(error) = result {
                let message = format!("Failed to reset game: {error}");
                log::error!("{message}");
                crate::ui::send(crate::ui::Message::FatalError(message));
            }
        });

        let state_ = state.clone();
        backend.on_game_exit(move || {
            // Cut cartridge power (if enabled)
            if let Err(error) = Device::lock().set_cart_power(false) {
                log::error!("Failed to disable cartridge power: {error:#}");
            }
            // Go back to the main menu
            let root = {
                let state = state_.borrow_mut();
                state.root.unwrap()
            };
            root.invoke_set_screen(ScreenId::MainMenu);
            // Reprogramming is blocking and can fail, so let the worker handle it.
            worker::send(worker::Message::EnsureBootBitstream);
        });
    }

    pub fn game_on_saved(&mut self) {
        // Ensure that the save icon is shown for a visible amount of time.
        let window = self.root.upgrade().unwrap();
        Timer::single_shot(Duration::from_millis(1000), move || {
            window.global::<Backend>().set_status_is_saving(false);
        });
    }
}
