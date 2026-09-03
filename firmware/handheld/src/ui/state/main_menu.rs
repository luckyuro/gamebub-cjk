use std::{cell::RefCell, rc::Rc};

use super::super::slint::Backend;
use slint::ComponentHandle;

use crate::{device::Device, worker};

use super::UiState;

impl UiState {
    /// Set up the "Main Menu" screen.
    pub(super) fn setup_main_menu(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        backend.on_main_menu_run_cartridge(|| worker::send(worker::Message::RunCartridge));

        let state_ = state.clone();
        backend.on_main_menu_load_rom(move || {
            let mut state = state_.borrow_mut();
            state.rom_select_load_saved_page();
            state.rom_select_update_path();
        });
    }
}
