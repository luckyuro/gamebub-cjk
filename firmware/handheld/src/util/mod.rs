pub mod background_io;

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn open_system_file(relative_path: &str) -> std::io::Result<File> {
    File::open(get_system_file_path(relative_path))
}

pub fn get_system_file_path(relative_path: &str) -> PathBuf {
    let path = Path::new("/sdcard/system/").join(relative_path);
    if path.is_file() {
        return path;
    }
    Path::new("/system/").join(relative_path)
}

#[allow(unused)]
pub fn copy_file(from: &Path, to: &Path) -> std::io::Result<u64> {
    let mut reader = std::fs::File::open(from)?;
    // Workaround for an issue with esp-idf
    let metadata = reader.metadata()?.file_type();

    if !metadata.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }

    let mut writer = std::fs::File::create(to)?;
    // `std::io::copy` uses an 8 KiB stack buffer on ESP-IDF. Save backups run on a
    // 16 KiB worker stack during ROM loading, so use a deliberately small bounded buffer.
    let mut buffer = [0u8; 1024];
    let mut total = 0u64;
    loop {
        let count = loop {
            match reader.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if count == 0 {
            return Ok(total);
        }
        writer.write_all(&buffer[..count])?;
        total += count as u64;
    }
}

/// Log allocator fragmentation and the current task's minimum remaining stack.
///
/// These checkpoints are deliberately allocation-free apart from the logger itself, so they can
/// still provide useful evidence immediately before a low-memory failure.
pub fn log_memory_checkpoint(label: &str) {
    unsafe {
        let caps = esp_idf_svc::sys::MALLOC_CAP_8BIT;
        let free_heap = esp_idf_svc::sys::heap_caps_get_free_size(caps);
        let largest_block = esp_idf_svc::sys::heap_caps_get_largest_free_block(caps);
        let stack_units = esp_idf_svc::sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut());
        let stack_bytes = (stack_units as usize)
            .saturating_mul(std::mem::size_of::<esp_idf_svc::sys::StackType_t>());
        log::info!(
            "Memory checkpoint {label}: heap_free={free_heap}, largest_block={largest_block}, stack_min_free={stack_bytes}"
        );
    }
}
