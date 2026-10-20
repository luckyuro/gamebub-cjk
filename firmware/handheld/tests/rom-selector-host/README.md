# Core file selector host regression and heap checks

This executable compiles the production Slint UI and includes the production
core file selector callbacks and directory selection algorithm. It uses real Slint key
handling, timers, ListView virtualization and a software renderer with a 360-pixel
line buffer. Only hardware access, worker I/O and core metadata are substituted.

From the repository root, with a host Rust toolchain (change the target on other hosts):

```sh
cargo +nightly run --manifest-path firmware/handheld/tests/rom-selector-host/Cargo.toml --target aarch64-apple-darwin
cargo +nightly run --manifest-path firmware/handheld/tests/rom-selector-host/Cargo.toml --target aarch64-apple-darwin --features=rom-list-scroll
rustc +nightly --edition=2021 --test firmware/handheld/src/rom_list.rs -o /tmp/rom-list-tests
/tmp/rom-list-tests
rustc +nightly --edition=2021 --cfg 'feature="rom-list-scroll"' --test firmware/handheld/src/rom_list.rs -o /tmp/rom-list-scroll-tests
/tmp/rom-list-scroll-tests
```

To check the actual bit-packed firmware font path, first prepare the Slint patch
with `build_fusion_pixel_full.sh`, then add both of these to each host build:

- Set `GAMEBUB_PACK_FUSION_PIXEL=1`.
- Pass `--config 'patch.crates-io.i-slint-core.path="/absolute/path/to/firmware/handheld/target/fusion-pixel-packed/i-slint-core-1.12.1"'`.

Always enable packing and the renderer patch together. The build script reuses
the firmware's exact coverage-text removal and bitmap packing code.

Checks cover B preserving the in-flight flag, blocked inputs not queueing more
work, immediate release of old model rows, 300 batches of 255-character filenames,
backward selection of the last file, cancellation of the selection timer on error,
navigation of an empty directory, ten-entry button pages with a separate parent row,
arrow/shoulder paging at the first and final pages, single-entry final pages,
Home cancellation during scans/timers, ignored
late replies, launch input gating, and saved-file/directory navigation using real
temporary files. Scroll mode sends approximately 9,600
Down key events and renders between them. The allocator records live requested
bytes and the maximum since warm-up, with a bounded growth assertion. The lazy
scanner also compares 1,000 and 100,000 long-name entries and checks all its
allocations are released.

These are host Rust allocation measurements, not the ESP32's total heap usage.
They exclude C/RTOS/driver allocations and malloc metadata/fragmentation. The
scanner's synthetic formatted Strings have spare capacity, making their retained
size larger than the exact-sized `to_string()` filenames used by the filesystem
reader. Real filesystem filtering and bidirectional scanning are covered by the
separate ten directory tests.

## Recorded review verification (2026-10-19)

Apple Silicon host, 360×240 UI, Slint 1.12.1, firmware bit-packed font patch:

| Check | Live bytes after warm-up | Maximum live bytes after warm-up | Peak bytes including temporary allocations |
| --- | ---: | ---: | ---: |
| Button paging, 300 batches | 132531 | 132563 | 161843 |
| Scrolling, 300 batches | 132705 | 132705 | 167354 |

The lazy directory scan retained 48288 bytes and peaked at 51972 bytes for both
1,000 and 100,000 synthetic long filenames; dropping the page returned the Rust
allocation count to baseline. All navigation/error assertions and the ten
standalone directory tests passed, including arbitrary core extension filters
and saved-file page selection.

Review fixes: preserve the backend loading flag when B returns to the parent;
reject selected/up callbacks during loading; reset the old FileListModel synchronously
before replacing it, including when launching a core; cancel the selection timer
when an error arrives or a core file starts loading.

Set `GAMEBUB_SIM_SCREENSHOT=/absolute/path/screen.ppm` to capture the first
360×240 frame, including the selected Chinese filename and paging hints.
The review also fixed initial focus when a page arrives before the file selector
component is created. CoreManager fault simulation is documented in
[core-host](../core-host/README.md).

Button paging also captures a sibling `screen.last.ppm` frame with the last entry
selected, to check that all ten entries and the parent row fit without scrolling.

## Paging update verification (2026-10-20)

The ten-entry paging build and 32-entry scrolling build each passed all ten
directory tests and 300 UI batches. Paging also passed arrow/shoulder navigation,
first/final-page limits, a one-entry final page, and parent-row counts. Both modes
had zero live allocation growth after warm-up. Paging retained 15112 bytes during
the lazy scan and peaked at 18796 bytes for both 1,000 and 100,000 filenames.
The 360×240 first/last selection renders show all ten entries and the parent row.
