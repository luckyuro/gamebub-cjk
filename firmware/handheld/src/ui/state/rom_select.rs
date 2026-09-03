use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use super::super::slint::Backend;
use super::super::slint::FileIcon;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{
    device::Device,
    kvs,
    rom_list::{RomListFocus, RomListPage, RomListPageRequest},
    worker,
};

use super::UiState;

pub const BASE_DIR: &str = "/sdcard/";

impl UiState {
    /// Set up the "Rom Select" screen.
    pub(super) fn setup_rom_select(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_rom_select_selected(move |index| {
            let mut state = state_.borrow_mut();
            let root = state.root.unwrap();
            let backend = root.global::<Backend>();
            let data = backend.get_rom_select_list().row_data(index as usize);
            if let Some(data) = data {
                match data.icon {
                    FileIcon::PreviousPage => {
                        if let Some(cursor) = state.rom_select_page_first.clone() {
                            state.rom_select_load_page(RomListPageRequest::Before(cursor));
                        }
                        false
                    }
                    FileIcon::NextPage => {
                        if let Some(cursor) = state.rom_select_page_last.clone() {
                            state.rom_select_load_page(RomListPageRequest::After(cursor));
                        }
                        false
                    }
                    FileIcon::Folder | FileIcon::Blank => {
                        let path = state.rom_select_directory.join(data.name.as_str());
                        state.rom_select_handle_select(
                            path,
                            data.name.as_str(),
                            data.icon == FileIcon::Folder,
                        )
                    }
                }
            } else {
                false
            }
        });

        let state_ = state.clone();
        backend.on_rom_select_up(move || {
            let mut state = state_.borrow_mut();
            state.rom_select_handle_select(PathBuf::new(), "..", true)
        });

        let state_ = state.clone();
        backend.on_rom_select_focused(move |index| {
            let state = state_.borrow_mut();
            let root = state.root.unwrap();
            let backend = root.global::<Backend>();
            let list = backend.get_rom_select_list();
            if let Some(data) = list.row_data(index as usize) {
                if matches!(data.icon, FileIcon::Folder | FileIcon::Blank) {
                    let path = state.rom_select_directory.join(data.name.as_str());
                    kvs::keys::LAST_ROM_PATH.set(&path);
                }
            }
        });
    }

    pub(super) fn rom_select_load_saved_page(&mut self) {
        let request = kvs::keys::LAST_ROM_PATH
            .get()
            .and_then(|path| path.file_name().map(|name| name.to_owned()))
            .and_then(|name| name.to_str().map(str::to_owned))
            .filter(|name| name != "..")
            .map(RomListPageRequest::ForName)
            .unwrap_or(RomListPageRequest::First);
        self.rom_select_load_page(request);
    }

    fn rom_select_load_page(&mut self, page: RomListPageRequest) {
        let path = self.rom_select_directory.clone();
        self.rom_select_page_first = None;
        self.rom_select_page_last = None;
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        // Drop the previous page before the worker allocates its replacement.
        backend.set_rom_select_list(ModelRc::default());
        backend.set_rom_select_index(-1);
        backend.set_rom_select_progress(0.0);
        backend.set_rom_select_is_loading(true);

        worker::send(worker::Message::ListRoms { path, page });
    }

    pub fn rom_select_update_list(&mut self, page: RomListPage) {
        let RomListPage {
            entries,
            has_previous,
            has_next,
            focus,
        } = page;
        self.rom_select_page_first = entries.first().cloned();
        self.rom_select_page_last = entries.last().cloned();

        let path = &self.rom_select_directory;
        let saved_name = kvs::keys::LAST_ROM_PATH
            .get()
            .and_then(|path| path.file_name().map(|name| name.to_owned()))
            .and_then(|name| name.to_str().map(str::to_owned));
        let mut selected_saved = None;
        let mut files = Vec::with_capacity(
            entries.len()
                + usize::from(path != Path::new(BASE_DIR))
                + usize::from(has_previous)
                + usize::from(has_next),
        );

        if path != Path::new(BASE_DIR) {
            if saved_name.as_deref() == Some("..") {
                selected_saved = Some(files.len());
            }
            files.push(crate::ui::slint::FileListEntry {
                name: "..".into(),
                icon: FileIcon::Folder,
            });
        }

        if has_previous {
            files.push(crate::ui::slint::FileListEntry {
                name: "Previous page".into(),
                icon: FileIcon::PreviousPage,
            });
        }

        let first_entry_index = files.len();
        let entry_count = entries.len();
        for entry in entries {
            if saved_name.as_deref() == Some(entry.name.as_str()) {
                selected_saved = Some(files.len());
            }
            files.push(crate::ui::slint::FileListEntry {
                name: entry.name.into(),
                icon: if entry.is_dir {
                    FileIcon::Folder
                } else {
                    FileIcon::Blank
                },
            });
        }

        if has_next {
            files.push(crate::ui::slint::FileListEntry {
                name: "Next page".into(),
                icon: FileIcon::NextPage,
            });
        }

        let first_entry = (entry_count > 0).then_some(first_entry_index);
        let last_entry = first_entry.map(|index| index + entry_count - 1);
        let selected = match focus {
            RomListFocus::Saved => selected_saved.or(first_entry),
            RomListFocus::First => first_entry,
            RomListFocus::Last => last_entry,
        }
        .unwrap_or(0);
        let files = ModelRc::from(Rc::new(VecModel::from(files)));

        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_list(files);
        self.rom_select_update_path();

        {
            // Defer setting the index and unsetting loading, because
            // the FileListView component can't handle shifting view to
            // a newly selected element until the first time it renders.
            let root = self.root.clone();
            self.rom_select_timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(1),
                move || {
                    let root = root.unwrap();
                    let backend = root.global::<Backend>();
                    backend.set_rom_select_index(selected as i32);
                    backend.set_rom_select_is_loading(false);
                },
            );
        }
    }

    pub fn rom_select_update_path(&self) {
        // Remove base directory from name before displaying.
        let path = &self.rom_select_directory;
        let mut directory = path
            .strip_prefix(BASE_DIR)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        if !directory.starts_with("/") {
            directory.insert_str(0, "/");
        }

        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_path(directory.into());
    }

    /// Handle selection. Returns whether a loading screen should be displayed.
    fn rom_select_handle_select(&mut self, path: PathBuf, filename: &str, is_dir: bool) -> bool {
        if filename == ".." {
            if self.rom_select_directory == Path::new(BASE_DIR) {
                log::warn!("No parent directory");
                return false;
            } else {
                kvs::keys::LAST_ROM_PATH.set(&self.rom_select_directory);
                self.rom_select_directory.pop();
                self.rom_select_load_saved_page();
            }
        } else if is_dir {
            log::info!("Entering subdirectory {}", filename);
            self.rom_select_directory.push(filename);
            let last_path = self.rom_select_directory.join("..");
            kvs::keys::LAST_ROM_PATH.set(&last_path);
            self.rom_select_load_saved_page();
        } else {
            log::info!("Selected ROM {}", path.display());
            kvs::keys::LAST_ROM_PATH.set(&path);
            let root = self.root.unwrap();
            let backend = root.global::<Backend>();
            backend.set_rom_select_list(ModelRc::default());
            backend.set_rom_select_progress(0.0);
            backend.set_rom_select_is_loading(true);
            worker::send(worker::Message::RunRomFile(path));
            return true;
        }
        false
    }

    pub fn rom_select_set_error(&mut self, error: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_is_loading(false);
        backend.set_rom_select_error(error.into());
        self.rom_select_update_path();
    }
}
