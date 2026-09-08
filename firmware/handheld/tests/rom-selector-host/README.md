# ROM selector host regression and heap checks

This executable compiles the production Slint UI and includes the production
ROM selector callbacks and directory selection algorithm. It uses real Slint key
handling, timers, ListView virtualization and a software renderer with a 320-pixel
line buffer. Only hardware access, worker I/O and KVS are substituted.

From the repository root, with a host Rust toolchain (change the target on other hosts):

```sh
cargo +nightly run --manifest-path firmware/handheld/tests/rom-selector-host/Cargo.toml --target aarch64-apple-darwin
cargo +nightly run --manifest-path firmware/handheld/tests/rom-selector-host/Cargo.toml --target aarch64-apple-darwin --features=rom-list-scroll
rustc +nightly --edition=2021 --test firmware/handheld/src/rom_list.rs -o /tmp/rom-list-tests
/tmp/rom-list-tests
```

To check the actual bit-packed firmware font path, first prepare the Slint patch
with `build_fusion_pixel_full.sh`, then add both of these to each host build:

- Set `GAMEBUB_PACK_FUSION_PIXEL=1`.
- Pass `--config 'patch.crates-io.i-slint-core.path="/absolute/path/to/firmware/handheld/target/fusion-pixel-packed/i-slint-core-1.12.1"'`.

Always enable packing and the renderer patch together. The build script reuses
the firmware's exact coverage-text removal and bitmap packing code.

Checks cover B preserving the in-flight flag, blocked inputs not queueing more
work, immediate release of old model rows, 300 batches of 255-character filenames,
backward selection of the last ROM, cancellation of the selection timer on error,
and navigation of an empty directory. Scroll mode sends approximately 9,600
Down key events and renders between them. The allocator records live requested
bytes and the maximum since warm-up, with a bounded growth assertion. The lazy
scanner also compares 1,000 and 100,000 long-name entries and checks all its
allocations are released.

These are host Rust allocation measurements, not the ESP32's total heap usage.
They exclude C/RTOS/driver allocations and malloc metadata/fragmentation. The
scanner's synthetic formatted Strings have spare capacity, making their retained
size larger than the exact-sized `to_string()` filenames used by the filesystem
reader. Real filesystem filtering and bidirectional scanning are covered by the
separate nine directory tests.

## Recorded review run (2026-09-08)

Apple Silicon host, 320×240 UI, Slint 1.12.1, firmware bit-packed font patch:

| Check | Live bytes after warm-up | Maximum live bytes after warm-up | Peak bytes including temporary allocations |
| --- | ---: | ---: | ---: |
| Button paging, 300 batches | 100541 | 100573 | 134621 |
| Scrolling, 300 batches | 99933 | 99933 | 134678 |

The lazy directory scan retained 48288 bytes and peaked at 51972 bytes for both
1,000 and 100,000 synthetic long filenames; dropping the page returned the Rust
allocation count to baseline. All navigation/error assertions and the nine
standalone directory tests passed. Unpacked-font runs also showed bounded memory
(32 bytes of post-warm-up growth for paging, zero for scrolling).

Review fixes: preserve the backend loading flag when B returns to the parent;
reject selected/up callbacks during loading; reset the old VecModel synchronously
before replacing it, including when launching a ROM; cancel the selection timer
when an error arrives or a ROM starts loading.
