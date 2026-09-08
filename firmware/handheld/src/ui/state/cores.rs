use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use super::super::slint::Backend;
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, SharedString, VecModel};

use crate::{
    device::Device,
    rom_list::{RomListFocus, RomListPage, RomListPageRequest},
    ui::slint::{CoreSubscreen, FileIcon, ScreenId},
    worker,
};

use super::UiState;

pub const BASE_DIR: &str = "/sdcard/";

struct FileListModel {
    data: RefCell<Vec<(String, FileIcon)>>,
    notify: ModelNotify,
}

impl Model for FileListModel {
    type Data = crate::ui::slint::FileListEntry;

    fn row_count(&self) -> usize {
        self.data.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        if row >= self.data.borrow().len() {
            return None;
        }
        let data = self.data.borrow();
        let (name, icon) = &data[row];
        Some(crate::ui::slint::FileListEntry {
            name: name.into(),
            icon: *icon,
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

impl UiState {
    /// Set up the "Cores" screen.
    pub(super) fn setup_cores(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        backend.set_core_file_select_scroll(cfg!(feature = "rom-list-scroll"));
        let state_ = state.clone();
        backend.on_core_file_select_page(move |next| {
            let mut state = state_.borrow_mut();
            let root = state.root.unwrap();
            let backend = root.global::<Backend>();
            if backend.get_core_file_select_is_loading() {
                return;
            }
            let cursor = if next && backend.get_core_file_select_has_next() {
                state.core_file_select_page_last.clone()
            } else if !next && backend.get_core_file_select_has_previous() {
                state.core_file_select_page_first.clone()
            } else {
                None
            };
            if let Some(cursor) = cursor {
                state.cores_file_select_load_page(if next {
                    RomListPageRequest::After(cursor)
                } else {
                    RomListPageRequest::Before(cursor)
                });
            }
        });

        backend.on_core_list_fetch(move || {
            worker::send(worker::Message::FetchCoreList);
        });

        let state_ = state.clone();
        backend.on_core_run(move |core_id| {
            let mut state = state_.borrow_mut();
            state.cores_handle_run(core_id);
        });

        let state_ = state.clone();
        backend.on_core_file_select_selected(move |index| {
            let mut state = state_.borrow_mut();
            let root: crate::ui::slint::MainWindow = state.root.unwrap();
            let backend = root.global::<Backend>();
            if backend.get_core_file_select_is_loading() || index < 0 {
                return;
            }
            let list = backend.get_core_file_select_list();
            if let Some(data) = list.row_data(index as usize) {
                match data.icon {
                    FileIcon::PreviousPage => {
                        if let Some(cursor) = state.core_file_select_page_first.clone() {
                            state.cores_file_select_load_page(RomListPageRequest::Before(cursor));
                        }
                    }
                    FileIcon::NextPage => {
                        if let Some(cursor) = state.core_file_select_page_last.clone() {
                            state.cores_file_select_load_page(RomListPageRequest::After(cursor));
                        }
                    }
                    _ => {
                        let path = state.core_file_select_directory.join(data.name.as_str());
                        state.cores_handle_file_select_selected(path, data.name.as_str());
                    }
                }
            }
        });

        let state_ = state.clone();
        backend.on_core_file_select_up(move || {
            let mut state = state_.borrow_mut();
            if state
                .root
                .unwrap()
                .global::<Backend>()
                .get_core_file_select_is_loading()
            {
                return;
            }
            state.cores_handle_file_select_selected(PathBuf::new(), "..");
        });

        let state_ = state.clone();
        backend.on_core_file_select_cancel(move || {
            let mut state = state_.borrow_mut();
            state.core_file_select_timer.stop();
            state.cores_file_select_clear_list();
            state.core_file_select_page_first = None;
            state.core_file_select_page_last = None;
            state
                .root
                .unwrap()
                .global::<Backend>()
                .set_core_file_select_is_loading(false);
            worker::send(worker::Message::CoreFileCancelled);
        });
    }

    pub fn cores_handle_run(&mut self, core_id: SharedString) {
        worker::send(worker::Message::RunCore(core_id.to_string()));
    }

    pub fn cores_list(&mut self, list: Vec<crate::core::CoreListEntry>) {
        let list = ModelRc::from(Rc::new(VecModel::from(
            list.into_iter()
                .map(|item| crate::ui::slint::CoreListEntry {
                    core_id: item.id.as_str().into(),
                    core_name: item.name.as_str().into(),
                    core_author: item.author.as_str().into(),
                })
                .collect::<Vec<_>>(),
        )));
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_list(list);
    }

    pub fn cores_file_select_begin(&mut self, label: String, path: PathBuf) {
        self.core_file_select_timer.stop();
        self.cores_file_select_clear_list();
        self.core_file_select_page_first = None;
        self.core_file_select_page_last = None;
        // `path` may be either a specific file (in which case
        // that file should be selected), or a directory.
        if path.is_file() {
            self.core_file_select_filename =
                path.file_name().unwrap().to_str().unwrap().to_string();
            self.core_file_select_directory = {
                let mut path = path;
                path.pop();
                path
            };
        } else {
            self.core_file_select_filename.clear();
            self.core_file_select_directory = path;
        }

        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        root.invoke_set_title(format!("Select {label}").into());
        backend.set_core_file_select_label(label.into());
        backend.set_core_file_select_error("".into());
        backend.set_core_file_select_has_previous(false);
        backend.set_core_file_select_has_next(false);
        backend.set_core_file_select_is_loading(true);
        self.cores_file_select_update_path();
        backend.set_core_subscreen(CoreSubscreen::FileSelect);
    }

    fn cores_file_select_clear_list(&self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        let model = backend.get_core_file_select_list();
        if let Some(model) = model.as_any().downcast_ref::<FileListModel>() {
            model.data.replace(Vec::new());
            model.notify.reset();
        }
        backend.set_core_file_select_list(ModelRc::default());
        backend.set_core_file_select_index(-1);
    }

    fn cores_file_select_load_page(&mut self, page: RomListPageRequest) {
        self.core_file_select_timer.stop();
        self.core_file_select_page_first = None;
        self.core_file_select_page_last = None;
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_file_select_is_loading(true);
        self.cores_file_select_clear_list();
        backend.set_core_file_select_has_previous(false);
        backend.set_core_file_select_has_next(false);
        backend.set_core_file_select_error("".into());
        self.cores_file_select_update_path();
        worker::send(worker::Message::CoreFileList {
            path: self.core_file_select_directory.clone(),
            page,
        });
    }

    pub fn cores_file_select_list(&mut self, page: RomListPage) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        if backend.get_core_subscreen() != CoreSubscreen::FileSelect {
            return;
        }
        let RomListPage {
            entries,
            has_previous,
            has_next,
            focus,
        } = page;
        self.core_file_select_page_first = entries.first().cloned();
        self.core_file_select_page_last = entries.last().cloned();
        let scroll = cfg!(feature = "rom-list-scroll");
        let mut files = Vec::with_capacity(entries.len() + 3);
        if self.core_file_select_directory != Path::new(BASE_DIR) && (!scroll || !has_previous) {
            files.push(("..".to_string(), FileIcon::Folder));
        }
        if has_previous && !scroll {
            files.push(("Previous page".to_string(), FileIcon::PreviousPage));
        }
        let first = files.len();
        let count = entries.len();
        for entry in entries {
            files.push((
                entry.name,
                if entry.is_dir {
                    FileIcon::Folder
                } else {
                    FileIcon::Blank
                },
            ));
        }
        if has_next && !scroll {
            files.push(("Next page".to_string(), FileIcon::NextPage));
        }
        let first_entry = (count > 0).then_some(first);
        let selected = match focus {
            RomListFocus::Saved => files
                .iter()
                .position(|(name, _)| *name == self.core_file_select_filename)
                .or(first_entry),
            RomListFocus::First => first_entry,
            RomListFocus::Last => first_entry.map(|first| first + count - 1),
        }
        .unwrap_or(0);
        backend.set_core_file_select_has_previous(has_previous);
        backend.set_core_file_select_has_next(has_next);
        backend.set_core_file_select_list(ModelRc::new(FileListModel {
            data: RefCell::new(files),
            notify: ModelNotify::default(),
        }));

        {
            // Defer setting the index and unsetting loading, because
            // the FileListView component can't handle shifting view to
            // a newly selected element until the first time it renders.
            let root = self.root.clone();
            self.core_file_select_timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(1),
                move || {
                    let root = root.unwrap();
                    let backend = root.global::<Backend>();
                    backend.set_core_file_select_index(selected as i32);
                    backend.set_core_file_select_is_loading(false);
                },
            );
        }
    }

    pub fn cores_file_select_update_path(&self) {
        // Remove base directory from name before displaying.
        let path = &self.core_file_select_directory;
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
        backend.set_core_file_select_path(directory.into());
    }

    pub fn cores_handle_file_select_selected(&mut self, path: PathBuf, filename: &str) {
        if filename == ".." {
            if self.core_file_select_directory == Path::new(BASE_DIR) {
                log::warn!("No parent directory");
                return;
            }

            // Highlight the directory we're just leaving.
            self.core_file_select_filename = self
                .core_file_select_directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            self.core_file_select_directory.pop();
            self.cores_file_select_load_page(RomListPageRequest::ForName(
                self.core_file_select_filename.clone(),
            ));
        } else if path.is_dir() {
            log::info!("Entering subdirectory {}", filename);
            self.core_file_select_filename.clear();
            self.core_file_select_directory.push(filename);
            self.cores_file_select_load_page(RomListPageRequest::First);
        } else {
            self.core_file_select_timer.stop();
            self.cores_file_select_clear_list();
            self.core_file_select_page_first = None;
            self.core_file_select_page_last = None;
            worker::send(worker::Message::CoreFileSelected(path));
        }

        self.root
            .unwrap()
            .global::<Backend>()
            .set_core_file_select_is_loading(true);
        self.cores_file_select_update_path();
    }

    pub fn cores_file_select_set_error(&mut self, error: String) {
        self.core_file_select_timer.stop();
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_file_select_is_loading(false);
        backend.set_core_file_select_error(error.into());
        self.cores_file_select_update_path();
    }

    pub fn cores_set_error(&mut self, error: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_core_error(error.into());
        backend.set_core_subscreen(CoreSubscreen::Error);
        root.invoke_set_screen(ScreenId::Cores);
    }
}
