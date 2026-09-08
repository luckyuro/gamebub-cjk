use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    fs::DirEntry,
    path::Path,
};

/// Bound the amount of directory data retained while building the ROM selector model.
// Shared by button paging and scrolling; never append pages to the UI model.
pub const PAGE_SIZE: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RomListEntry {
    pub name: String,
    pub is_dir: bool,
}

impl Ord for RomListEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Match the existing UI order: directories first, then names.
        (!self.is_dir, self.name.as_str()).cmp(&(!other.is_dir, other.name.as_str()))
    }
}

impl PartialOrd for RomListEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug)]
pub enum RomListPageRequest {
    /// Load the first page, selecting the last saved entry if it is present.
    First,
    /// Load the page beginning with this entry, or the first page if it disappeared.
    ForName(String),
    /// Load the page immediately after this entry.
    After(RomListEntry),
    /// Load the page immediately before this entry.
    Before(RomListEntry),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RomListFocus {
    Saved,
    First,
    Last,
}

#[derive(Debug)]
pub struct RomListPage {
    pub entries: Vec<RomListEntry>,
    pub has_previous: bool,
    pub has_next: bool,
    pub focus: RomListFocus,
}

enum Selection {
    First,
    AtOrAfter(RomListEntry),
    After(RomListEntry),
    Before(RomListEntry),
}

fn is_supported_rom(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("gb")
                || extension.eq_ignore_ascii_case("gbc")
                || extension.eq_ignore_ascii_case("gba")
        })
}

fn eligible_entry(entry: std::io::Result<DirEntry>) -> Option<RomListEntry> {
    let entry = entry.ok()?;
    let name = entry.file_name();
    let name = name.to_str()?;
    let kind = entry.file_type().ok()?;

    if name.starts_with('.') || (kind.is_file() && !is_supported_rom(name)) {
        return None;
    }

    Some(RomListEntry {
        name: name.to_string(),
        is_dir: kind.is_dir(),
    })
}

fn eligible_entries(path: &Path) -> std::io::Result<impl Iterator<Item = RomListEntry> + use<>> {
    Ok(path.read_dir()?.filter_map(eligible_entry))
}

fn select_forward(
    entries: impl Iterator<Item = RomListEntry>,
    selection: Selection,
    focus: RomListFocus,
) -> RomListPage {
    let mut candidates = BinaryHeap::with_capacity(PAGE_SIZE + 1);
    let mut has_previous = false;

    for entry in entries {
        let include = match &selection {
            Selection::First => true,
            Selection::AtOrAfter(cursor) => entry >= *cursor,
            Selection::After(cursor) => entry > *cursor,
            Selection::Before(_) => unreachable!(),
        };
        if !include {
            has_previous = true;
            continue;
        }

        if candidates.len() < PAGE_SIZE + 1 {
            candidates.push(entry);
        } else if candidates.peek().is_some_and(|largest| entry < *largest) {
            *candidates.peek_mut().unwrap() = entry;
        }
    }

    let mut entries = candidates.into_sorted_vec();
    let has_next = entries.len() > PAGE_SIZE;
    entries.truncate(PAGE_SIZE);
    RomListPage {
        entries,
        has_previous,
        has_next,
        focus,
    }
}

fn select_backward(
    entries: impl Iterator<Item = RomListEntry>,
    cursor: RomListEntry,
) -> RomListPage {
    let mut candidates = BinaryHeap::with_capacity(PAGE_SIZE + 1);
    let mut has_next = false;

    for entry in entries {
        if entry >= cursor {
            has_next = true;
            continue;
        }

        let entry = Reverse(entry);
        if candidates.len() < PAGE_SIZE + 1 {
            candidates.push(entry);
        } else if candidates.peek().is_some_and(|smallest| entry < *smallest) {
            *candidates.peek_mut().unwrap() = entry;
        }
    }

    let mut entries = candidates
        .into_iter()
        .map(|entry| entry.0)
        .collect::<Vec<_>>();
    entries.sort_unstable();
    let has_previous = entries.len() > PAGE_SIZE;
    if has_previous {
        entries.remove(0);
    }
    RomListPage {
        entries,
        has_previous,
        has_next,
        focus: RomListFocus::Last,
    }
}

fn select_entries(
    entries: impl Iterator<Item = RomListEntry>,
    selection: Selection,
    focus: RomListFocus,
) -> RomListPage {
    match selection {
        Selection::Before(cursor) => select_backward(entries, cursor),
        selection => select_forward(entries, selection, focus),
    }
}

/// Read one sorted, bounded page from a directory.
///
/// Directory iteration remains O(n), but retained memory is O(PAGE_SIZE) instead of O(n).
pub fn read_rom_list_page(
    path: &Path,
    request: RomListPageRequest,
) -> std::io::Result<RomListPage> {
    let (selection, focus) = match request {
        RomListPageRequest::First => (Selection::First, RomListFocus::Saved),
        RomListPageRequest::ForName(name) => {
            let entry = eligible_entries(path)?.find(|entry| entry.name == name);
            match entry {
                Some(entry) => (Selection::AtOrAfter(entry), RomListFocus::Saved),
                None => (Selection::First, RomListFocus::Saved),
            }
        }
        RomListPageRequest::After(entry) => (Selection::After(entry), RomListFocus::First),
        RomListPageRequest::Before(entry) => (Selection::Before(entry), RomListFocus::Last),
    };
    Ok(select_entries(eligible_entries(path)?, selection, focus))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, File},
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let id = NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "gamebub-rom-list-test-{}-{nonce}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn files(count: usize) -> Vec<RomListEntry> {
        (0..count)
            .rev()
            .map(|index| RomListEntry {
                name: format!("file-{index:04}.gba"),
                is_dir: false,
            })
            .collect()
    }

    #[test]
    fn first_page_is_sorted_and_bounded() {
        let page = select_entries(
            files(PAGE_SIZE * 4).into_iter(),
            Selection::First,
            RomListFocus::Saved,
        );

        assert_eq!(page.entries.len(), PAGE_SIZE);
        assert_eq!(page.entries[0].name, "file-0000.gba");
        assert_eq!(
            page.entries[PAGE_SIZE - 1].name,
            format!("file-{:04}.gba", PAGE_SIZE - 1)
        );
        assert!(!page.has_previous);
        assert!(page.has_next);
    }

    #[test]
    fn large_lazy_input_still_returns_one_bounded_page() {
        let entries = (0..100_000).rev().map(|index| RomListEntry {
            name: format!("file-{index:06}.gba"),
            is_dir: false,
        });
        let page = select_entries(entries, Selection::First, RomListFocus::Saved);

        assert_eq!(page.entries.len(), PAGE_SIZE);
        assert_eq!(page.entries[0].name, "file-000000.gba");
        assert_eq!(
            page.entries[PAGE_SIZE - 1].name,
            format!("file-{:06}.gba", PAGE_SIZE - 1)
        );
        assert!(page.has_next);
    }

    #[test]
    fn next_and_previous_pages_are_contiguous() {
        let all_files = files(PAGE_SIZE * 3);
        let first = select_entries(
            all_files.clone().into_iter(),
            Selection::First,
            RomListFocus::Saved,
        );
        let second = select_entries(
            all_files.clone().into_iter(),
            Selection::After(first.entries.last().unwrap().clone()),
            RomListFocus::First,
        );
        let previous = select_entries(
            all_files.into_iter(),
            Selection::Before(second.entries.first().unwrap().clone()),
            RomListFocus::Last,
        );

        assert_eq!(second.entries[0].name, format!("file-{:04}.gba", PAGE_SIZE));
        assert_eq!(first.entries, previous.entries);
        assert_eq!(second.focus, RomListFocus::First);
        assert_eq!(previous.focus, RomListFocus::Last);
    }

    #[test]
    fn saved_entry_begins_its_page() {
        let all_files = files(PAGE_SIZE * 3);
        let saved = all_files
            .iter()
            .find(|entry| entry.name == format!("file-{:04}.gba", PAGE_SIZE + 22))
            .unwrap()
            .clone();
        let page = select_entries(
            all_files.into_iter(),
            Selection::AtOrAfter(saved.clone()),
            RomListFocus::Saved,
        );

        assert_eq!(page.entries.first(), Some(&saved));
        assert_eq!(page.entries.len(), PAGE_SIZE);
        assert!(page.has_previous);
        assert!(page.has_next);
    }

    #[test]
    fn directories_sort_before_roms() {
        let page = select_entries(
            vec![
                RomListEntry {
                    name: "a.gba".into(),
                    is_dir: false,
                },
                RomListEntry {
                    name: "z".into(),
                    is_dir: true,
                },
                RomListEntry {
                    name: "a".into(),
                    is_dir: true,
                },
            ]
            .into_iter(),
            Selection::First,
            RomListFocus::Saved,
        );

        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "z", "a.gba"]
        );
    }

    #[test]
    fn directory_reader_filters_and_pages_real_entries() {
        let directory = TempDirectory::new();
        fs::create_dir(directory.0.join("folder")).unwrap();
        File::create(directory.0.join("notes.txt")).unwrap();
        File::create(directory.0.join(".hidden.gba")).unwrap();
        for index in 0..PAGE_SIZE + 5 {
            File::create(directory.0.join(format!("rom-{index:04}.gba"))).unwrap();
        }

        let first = read_rom_list_page(&directory.0, RomListPageRequest::First).unwrap();
        assert_eq!(first.entries.len(), PAGE_SIZE);
        assert_eq!(first.entries[0].name, "folder");
        assert!(!first.has_previous);
        assert!(first.has_next);
        assert!(first.entries.iter().all(|entry| entry.name != "notes.txt"));
        assert!(first
            .entries
            .iter()
            .all(|entry| entry.name != ".hidden.gba"));

        let next = read_rom_list_page(
            &directory.0,
            RomListPageRequest::After(first.entries.last().unwrap().clone()),
        )
        .unwrap();
        assert_eq!(next.entries.len(), 6);
        assert!(next.has_previous);
        assert!(!next.has_next);

        let saved = read_rom_list_page(
            &directory.0,
            RomListPageRequest::ForName(format!("rom-{:04}.gba", PAGE_SIZE + 2)),
        )
        .unwrap();
        assert_eq!(
            saved.entries[0].name,
            format!("rom-{:04}.gba", PAGE_SIZE + 2)
        );
        assert_eq!(saved.focus, RomListFocus::Saved);
    }

    #[test]
    fn repeated_bidirectional_navigation_does_not_skip_long_cjk_names() {
        let directory = TempDirectory::new();
        // Long UTF-8 names exercise filename allocation as well as page boundaries.
        let prefix = "中文游戏".repeat(18);
        let count = PAGE_SIZE * 5 + 3;
        for index in 0..count {
            File::create(directory.0.join(format!("{prefix}-{index:04}.gba"))).unwrap();
        }
        for _ in 0..3 {
            let mut request = RomListPageRequest::First;
            let mut seen = 0;
            let last = loop {
                let page = read_rom_list_page(&directory.0, request).unwrap();
                assert!(page.entries.len() <= PAGE_SIZE);
                for entry in &page.entries {
                    assert_eq!(entry.name, format!("{prefix}-{seen:04}.gba"));
                    seen += 1;
                }
                let cursor = page.entries.last().unwrap().clone();
                if !page.has_next {
                    break page.entries.first().unwrap().clone();
                }
                request = RomListPageRequest::After(cursor);
            };
            assert_eq!(seen, count);
            let mut cursor = last;
            let mut expected = count - 3;
            while expected > 0 {
                let page =
                    read_rom_list_page(&directory.0, RomListPageRequest::Before(cursor)).unwrap();
                assert!(page.entries.len() <= PAGE_SIZE);
                for entry in page.entries.iter().rev() {
                    expected -= 1;
                    assert_eq!(entry.name, format!("{prefix}-{expected:04}.gba"));
                }
                cursor = page.entries.first().unwrap().clone();
                assert_eq!(page.has_previous, expected > 0);
            }
        }
    }

    #[test]
    fn empty_and_exact_page_directories_have_no_next_page() {
        let directory = TempDirectory::new();
        let empty = read_rom_list_page(&directory.0, RomListPageRequest::First).unwrap();
        assert!(empty.entries.is_empty());
        assert!(!empty.has_previous && !empty.has_next);
        let page = select_entries(
            files(PAGE_SIZE).into_iter(),
            Selection::First,
            RomListFocus::Saved,
        );
        assert_eq!(page.entries.len(), PAGE_SIZE);
        assert!(!page.has_previous && !page.has_next);
    }

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(is_supported_rom("Pokemon.GBA"));
        assert!(is_supported_rom("Pokemon.GbC"));
        assert!(is_supported_rom("Pokemon.gb"));
        assert!(!is_supported_rom("Pokemon.gba.txt"));
    }
}
