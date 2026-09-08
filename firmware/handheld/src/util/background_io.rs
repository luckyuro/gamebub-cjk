use std::{fs::File, io::Read, time::Instant};

/// Read a file in bounded chunks and pass each chunk to the consumer.
///
/// This intentionally runs on the caller's existing worker thread. Creating a temporary
/// scoped thread for every ROM load requires another FreeRTOS stack allocation precisely
/// when memory pressure is highest, and `Scope::spawn` cannot report allocation failure.
pub fn iter_chunks<E>(
    mut file: File,
    buffer: &mut [u8],
    mut consume: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<std::io::Error>,
{
    if buffer.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ROM read buffer is empty",
        )
        .into());
    }

    let mut read_duration = std::time::Duration::ZERO;
    loop {
        let read_start = Instant::now();
        let read = loop {
            match file.read(buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => break result,
            }
        };
        read_duration += read_start.elapsed();

        let count = read.map_err(E::from)?;
        if count == 0 {
            log::info!("Read in {}ms", read_duration.as_millis() as u32);
            return Ok(());
        }
        consume(&buffer[..count])?;
    }
}
