# Core lifecycle fault simulation

This host test compiles the production CoreManager, directory scanner, chunked
file reader, and scratch buffer. Only the device/SPI, Slint message delivery and
GB/GBA hardware callbacks are replaced. It is not an FPGA or ESP32 emulator.

```sh
cargo +nightly test --manifest-path firmware/handheld/tests/core-host/Cargo.toml --target aarch64-apple-darwin
python3 firmware/handheld/tests/test_make_uf2.py
```

The 26 Rust tests cover ten directory cases, thirteen lifecycle/I/O cases and
three BIOS selection cases:
late cancellation after launch, cancellation cleanup, disappeared files, queued
selection after cancellation, duplicate launch, a selected ROM disappearing
before open, command read/write faults, exact chunked transfer, data transfer
failure, save-memory initialization failure, consumer failure and empty read
buffers, and filtered directory/error delivery. Device faults must be returned
without a panic or a false success, and scratch memory must be released.

BIOS checks use real temporary files to verify SD/bundled selection for all four
filenames, disabling and re-enabling without deleting files, missing files,
wrong sizes, directory paths, read errors, independent GB/GBA switches and the
GBA fast-boot filename. The real KVS key definitions use an in-memory store on the
host; NVS persistence still needs a device check.

The eight lifecycle regressions initially failed on rebased commit `a4a9be3` and
passed after the review fixes. The UF2 tests independently decode every block,
reconstruct both partitions, check device/commit tags and address boundaries,
and reject missing system data or oversized application images.

The real Slint input/rendering stress test is in `../rom-selector-host`. Hardware
startup, actual SPI timing, SD-card driver behavior and in-game compatibility
still require a physical rev4 device.

Validate generated rev4 bundles with the production DFU parser:

```sh
rustc +nightly --edition=2021 firmware/handheld/tests/validate_uf2.rs -o /tmp/validate-uf2
/tmp/validate-uf2 SOURCE_COMMIT gamebub-cjk-rev4-paged.uf2 gamebub-cjk-rev4-scroll.uf2
```

`SOURCE_COMMIT` is the firmware source commit in `uf2-build-info.json`, which
precedes the commit adding the generated binaries.
