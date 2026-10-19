//! Headless integration/stress check using the real UI and core file selector callbacks.
//! Hardware, worker I/O and core metadata are replaced; rendering and input are real Slint.
use slint::platform::{
    software_renderer::{
        LineBufferProvider, MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel,
    },
    Platform, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, Model};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct CountAlloc;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let n = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(p, layout);
    }
    unsafe fn realloc(&self, p: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let new = System.realloc(p, old, size);
        if !new.is_null() {
            LIVE.fetch_sub(old.size(), Ordering::Relaxed);
            let n = LIVE.fetch_add(size, Ordering::Relaxed) + size;
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        new
    }
}
#[global_allocator]
static ALLOC: CountAlloc = CountAlloc;
#[allow(dead_code)]
mod rom_list {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/rom_list.rs"
    ));

    pub fn memory_scan(count: usize) -> RomListPage {
        select_entries(
            (0..count).rev().map(|i| RomListEntry {
                name: format!("{i:06}{}中.gba", "中文游戏".repeat(61)),
                is_dir: false,
            }),
            Selection::First,
            RomListFocus::First,
        )
    }
}
mod device {
    pub struct Device;
}
mod core {
    pub struct CoreListEntry {
        pub id: String,
        pub name: String,
        pub author: String,
    }
}
mod worker {
    use super::*;
    #[derive(Debug)]
    pub enum Message {
        FetchCoreList,
        RunCore(String),
        CoreFileCancelled,
        CoreFileSelected(std::path::PathBuf),
        CoreFileList {
            path: std::path::PathBuf,
            page: rom_list::RomListPageRequest,
        },
    }
    thread_local! { static QUEUE: RefCell<Vec<Message>> = const { RefCell::new(Vec::new()) }; }
    pub fn send(message: Message) {
        QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            assert!(q.is_empty(), "overlapping worker requests");
            q.push(message);
        });
    }
    pub fn take() -> Option<Message> {
        QUEUE.with(|q| q.borrow_mut().pop())
    }
}
mod ui {
    pub mod slint {
        slint::include_modules!();
    }
    pub mod state {
        use super::slint::MainWindow;
        use crate::{device::Device, rom_list::RomListEntry};
        use std::{cell::RefCell, path::PathBuf, rc::Rc};
        pub struct UiState {
            root: slint::Weak<MainWindow>,
            core_file_select_directory: PathBuf,
            core_file_select_filename: String,
            core_file_select_page_first: Option<RomListEntry>,
            core_file_select_page_last: Option<RomListEntry>,
            core_file_select_timer: slint::Timer,
        }
        mod cores {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../src/ui/state/cores.rs"
            ));
        }
        impl UiState {
            pub fn fixture(root: &MainWindow) -> Rc<RefCell<Self>> {
                let state = Rc::new(RefCell::new(Self {
                    root: root.as_weak(),
                    core_file_select_directory: "/sdcard/child".into(),
                    core_file_select_filename: String::new(),
                    core_file_select_page_first: None,
                    core_file_select_page_last: None,
                    core_file_select_timer: Default::default(),
                }));
                state.borrow_mut().setup_cores(&state, &mut Device);
                state
            }
            pub fn first(&mut self) {
                self.cores_file_select_begin("ROM".into(), self.core_file_select_directory.clone());
                crate::worker::send(crate::worker::Message::CoreFileList {
                    path: self.core_file_select_directory.clone(),
                    page: crate::rom_list::RomListPageRequest::First,
                });
            }
        }
        use slint::ComponentHandle;
    }
}
thread_local! { static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) }; }
struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        NOW.with(Cell::get)
    }
}
struct Lines([Rgb565Pixel; 360]);
impl LineBufferProvider for &mut Lines {
    type TargetPixel = Rgb565Pixel;
    fn process_line(
        &mut self,
        _: usize,
        range: std::ops::Range<usize>,
        render: impl FnOnce(&mut [Rgb565Pixel]),
    ) {
        render(&mut self.0[range]);
    }
}
fn frame(window: &MinimalSoftwareWindow) {
    NOW.with(|t| t.set(t.get() + Duration::from_millis(10)));
    slint::platform::update_timers_and_animations();
    window.draw_if_needed(|r| {
        r.render_by_line(&mut Lines([Rgb565Pixel::default(); 360]));
    });
}
fn snapshot(window: &MinimalSoftwareWindow) {
    let Some(path) = std::env::var_os("GAMEBUB_SIM_SCREENSHOT") else {
        return;
    };
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        let mut pixels = vec![Rgb565Pixel::default(); 360 * 240];
        renderer.render(&mut pixels, 360);
        let mut ppm = b"P6\n360 240\n255\n".to_vec();
        for pixel in pixels {
            let r = ((pixel.0 >> 11) & 31) as u8;
            let g = ((pixel.0 >> 5) & 63) as u8;
            let b = (pixel.0 & 31) as u8;
            ppm.extend_from_slice(&[
                (r << 3) | (r >> 2),
                (g << 2) | (g >> 4),
                (b << 3) | (b >> 2),
            ]);
        }
        std::fs::write(path, ppm).unwrap();
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
    });
}
fn key(window: &MinimalSoftwareWindow, text: slint::SharedString) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}
fn page(start: usize, total: usize, focus: rom_list::RomListFocus) -> rom_list::RomListPage {
    rom_list::RomListPage {
        entries: (start..(start + rom_list::PAGE_SIZE).min(total))
            .map(|i| rom_list::RomListEntry {
                // FAT LFN maximum is 255 UTF-16 units; use 255 BMP characters.
                name: format!("{i:06}{}{}.gba", "中文游戏".repeat(61), "中"),
                is_dir: false,
            })
            .collect(),
        has_previous: start > 0,
        has_next: start + rom_list::PAGE_SIZE < total,
        focus,
    }
}
fn finish(
    state: &Rc<RefCell<ui::state::UiState>>,
    window: &MinimalSoftwareWindow,
    start: usize,
    total: usize,
    focus: rom_list::RomListFocus,
) {
    state
        .borrow_mut()
        .cores_file_select_list(page(start, total, focus));
    frame(window);
    frame(window);
}
fn main() {
    for count in [1_000, 100_000] {
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        let scanned = rom_list::memory_scan(count);
        let retained = LIVE.load(Ordering::Relaxed) - baseline;
        let peak = PEAK.load(Ordering::Relaxed) - baseline;
        assert_eq!(scanned.entries.len(), rom_list::PAGE_SIZE);
        assert!(peak < 64 * 1024, "directory scan exceeded heap budget");
        drop(scanned);
        assert_eq!(
            LIVE.load(Ordering::Relaxed),
            baseline,
            "scan leaked allocations"
        );
        println!("scan count={count} retained={retained} peak={peak}");
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let root = ui::slint::MainWindow::new().unwrap();
    root.window().set_size(slint::PhysicalSize::new(360, 240));
    root.show().unwrap();
    let state = ui::state::UiState::fixture(&root);
    root.invoke_set_screen(ui::slint::ScreenId::Cores);
    frame(&window);
    assert!(matches!(
        worker::take(),
        Some(worker::Message::FetchCoreList)
    ));
    let backend = root.global::<ui::slint::Backend>();
    state.borrow_mut().cores_list(vec![core::CoreListEntry {
        id: "Game-Bub.GBA".into(),
        name: "Game Boy Advance".into(),
        author: "Game Bub".into(),
    }]);
    frame(&window);
    key(&window, "a".into());
    assert!(matches!(worker::take(), Some(worker::Message::RunCore(id)) if id == "Game-Bub.GBA"));
    key(&window, "b".into());
    key(&window, "a".into());
    backend.invoke_core_run("Game-Bub.GBA".into());
    assert!(
        worker::take().is_none(),
        "core launch allowed overlapping input"
    );
    assert_eq!(backend.get_core_subscreen(), ui::slint::CoreSubscreen::List);
    state.borrow_mut().first();
    assert!(matches!(
        worker::take(),
        Some(worker::Message::CoreFileList { .. })
    ));
    finish(&state, &window, 0, 10_000, rom_list::RomListFocus::First);
    snapshot(&window);

    // B must keep loading true and suppress further actions until the response.
    key(&window, "b".into());
    assert!(
        backend.get_core_file_select_is_loading(),
        "B cleared the in-flight flag"
    );
    let request = worker::take().expect("B must request parent directory");
    assert!(
        matches!(request, worker::Message::CoreFileList { ref path, .. } if path == std::path::Path::new("/sdcard"))
    );
    key(&window, "b".into());
    key(&window, "a".into());
    backend.invoke_core_file_select_up();
    backend.invoke_core_file_select_selected(0);
    backend.invoke_core_file_select_page(true);
    assert!(worker::take().is_none(), "input queued work while loading");
    finish(&state, &window, 0, 10_000, rom_list::RomListFocus::First);

    let mut warm_live = 0;
    let mut maximum_live = 0;
    // Real key events, list virtualization, font shaping and line rendering.
    for batch in 0..300 {
        let old_model = backend.get_core_file_select_list();
        if cfg!(feature = "rom-list-scroll") {
            for _ in 0..rom_list::PAGE_SIZE {
                key(&window, slint::platform::Key::DownArrow.into());
                frame(&window);
            }
        } else {
            key(&window, slint::platform::Key::RightArrow.into());
        }
        let request = worker::take().expect("navigation must load next batch");
        match request {
            worker::Message::CoreFileList {
                page: rom_list::RomListPageRequest::After(cursor),
                ..
            } => assert!(cursor.name.starts_with(&format!("{:06}", batch * 32 + 31))),
            other => panic!("unexpected request: {other:?}"),
        }
        assert!(backend.get_core_file_select_is_loading());
        assert_eq!(backend.get_core_file_select_list().row_count(), 0);
        assert_eq!(old_model.row_count(), 0, "old model retains filenames");
        drop(old_model);
        finish(
            &state,
            &window,
            (batch + 1) * 32,
            10_000,
            rom_list::RomListFocus::First,
        );
        let live = LIVE.load(Ordering::Relaxed);
        if batch == 10 {
            warm_live = live;
            PEAK.store(live, Ordering::Relaxed);
        }
        if batch > 10 {
            maximum_live = maximum_live.max(live);
        }
    }
    let peak = PEAK.load(Ordering::Relaxed);
    println!(
        "mode={} warm_live={} max_live={} peak={} growth={}",
        if cfg!(feature = "rom-list-scroll") {
            "scroll"
        } else {
            "paged"
        },
        warm_live,
        maximum_live,
        peak,
        maximum_live.saturating_sub(warm_live)
    );
    assert!(
        maximum_live <= warm_live + 16 * 1024,
        "heap grows across batches"
    );

    // Previous batch must focus its last ROM; scroll Up crosses immediately.
    key(
        &window,
        if cfg!(feature = "rom-list-scroll") {
            slint::platform::Key::UpArrow.into()
        } else {
            slint::platform::Key::LeftArrow.into()
        },
    );
    assert!(matches!(
        worker::take(),
        Some(worker::Message::CoreFileList {
            page: rom_list::RomListPageRequest::Before(_),
            ..
        })
    ));
    finish(
        &state,
        &window,
        299 * 32,
        10_000,
        rom_list::RomListFocus::Last,
    );
    key(&window, "a".into());
    assert!(
        matches!(worker::take(), Some(worker::Message::CoreFileSelected(path)) if path.file_name().unwrap().to_str().unwrap().starts_with("009599"))
    );
    // An error must cancel the pending selection timer.
    root.invoke_set_screen(ui::slint::ScreenId::Cores);
    state.borrow_mut().first();
    worker::take().unwrap();
    state
        .borrow_mut()
        .cores_file_select_list(page(0, 32, rom_list::RomListFocus::First));
    state
        .borrow_mut()
        .cores_file_select_set_error("test read error".into());
    frame(&window);
    frame(&window);
    assert_eq!(backend.get_core_file_select_index(), -1);
    assert_eq!(backend.get_core_file_select_error(), "test read error");

    // Empty directory: no selection, division/modulo panic or worker request.
    state.borrow_mut().first();
    worker::take().unwrap();
    finish(&state, &window, 0, 0, rom_list::RomListFocus::First);
    key(&window, slint::platform::Key::UpArrow.into());
    key(&window, slint::platform::Key::DownArrow.into());
    key(&window, "a".into());
    frame(&window);
    assert!(worker::take().is_none());
    // Home cancels a pending scan; its late page/error must not reopen or mutate the selector.
    state.borrow_mut().first();
    worker::take().unwrap();
    frame(&window);
    key(&window, "Home".into());
    assert!(matches!(
        worker::take(),
        Some(worker::Message::CoreFileCancelled)
    ));
    assert_eq!(backend.get_core_subscreen(), ui::slint::CoreSubscreen::List);
    finish(&state, &window, 0, 100, rom_list::RomListFocus::First);
    state
        .borrow_mut()
        .cores_file_select_set_error("late read error".into());
    assert_eq!(backend.get_core_file_select_list().row_count(), 0);
    assert_eq!(backend.get_core_file_select_error(), "");
    assert_eq!(backend.get_core_file_select_index(), -1);
    assert!(!backend.get_core_file_select_is_loading());

    // Home also cancels a page's deferred selection timer.
    state.borrow_mut().first();
    worker::take().unwrap();
    state
        .borrow_mut()
        .cores_file_select_list(page(0, 32, rom_list::RomListFocus::First));
    backend.set_core_subscreen(ui::slint::CoreSubscreen::List);
    backend.invoke_core_file_select_cancel();
    assert!(matches!(
        worker::take(),
        Some(worker::Message::CoreFileCancelled)
    ));
    frame(&window);
    assert_eq!(backend.get_core_file_select_index(), -1);

    // Exercise real directory reads, saved selection and directory entry/return.
    let dir = std::env::temp_dir().join(format!("gamebub-ui-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("child")).unwrap();
    for i in 0..70 {
        std::fs::write(dir.join(format!("中文游戏-{i:03}.gba")), b"test").unwrap();
    }
    let saved = "中文游戏-050.gba";
    state
        .borrow_mut()
        .cores_file_select_begin("ROM".into(), dir.join(saved));
    let saved_page = rom_list::read_file_list_page(
        &dir,
        rom_list::RomListPageRequest::ForName(saved.into()),
        &[".gba"],
    )
    .unwrap();
    state.borrow_mut().cores_file_select_list(saved_page);
    frame(&window);
    frame(&window);
    let index = backend.get_core_file_select_index();
    assert_eq!(
        backend
            .get_core_file_select_list()
            .row_data(index as usize)
            .unwrap()
            .name,
        saved
    );
    key(&window, "a".into());
    let selected = worker::take();
    assert!(
        matches!(&selected, Some(worker::Message::CoreFileSelected(path)) if *path == dir.join(saved)),
        "saved selection emitted {selected:?}; backend index={index}"
    );

    state
        .borrow_mut()
        .cores_file_select_begin("ROM".into(), dir.clone());
    let first = rom_list::read_file_list_page(&dir, rom_list::RomListPageRequest::First, &[".gba"])
        .unwrap();
    state.borrow_mut().cores_file_select_list(first);
    frame(&window);
    frame(&window);
    key(&window, "a".into());
    let child = match worker::take().unwrap() {
        worker::Message::CoreFileList {
            path,
            page: rom_list::RomListPageRequest::First,
        } => path,
        other => panic!("unexpected directory selection: {other:?}"),
    };
    assert_eq!(child, dir.join("child"));
    state.borrow_mut().cores_file_select_list(
        rom_list::read_file_list_page(&child, rom_list::RomListPageRequest::First, &[".gba"])
            .unwrap(),
    );
    frame(&window);
    frame(&window);
    key(&window, "b".into());
    assert!(
        matches!(worker::take(), Some(worker::Message::CoreFileList { path, page: rom_list::RomListPageRequest::ForName(name) }) if path == dir && name == "child")
    );
    std::fs::remove_dir_all(&dir).unwrap();
    state
        .borrow_mut()
        .cores_file_select_set_error("directory removed".into());
    assert!(!backend.get_core_file_select_is_loading());
    key(&window, "b".into());
    assert_eq!(backend.get_core_file_select_error(), "");
    println!("navigation, cancellation, late replies, real filesystem, saved focus, empty/error and bounded-heap checks passed");
}
