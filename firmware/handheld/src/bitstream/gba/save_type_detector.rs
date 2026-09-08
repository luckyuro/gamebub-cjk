use super::SaveType;

/// Helper struct to auto-detect save file types from a ROM file (streaming)
pub struct SaveTypeDetector {
    detected: Option<SaveType>,
    buffer: Vec<u8>,
}

impl SaveTypeDetector {
    const OVERLAP: usize = 16;
    const STEP: usize = 4;

    pub fn new() -> Self {
        SaveTypeDetector {
            detected: None,
            buffer: vec![],
        }
    }

    pub fn get(&self) -> SaveType {
        self.detected.unwrap_or_default()
    }

    fn search(data: &[u8]) -> Option<SaveType> {
        static PREFIXES: &[u32] = &[
            u32::from_ne_bytes(*b"EEPR"),
            u32::from_ne_bytes(*b"SRAM"),
            u32::from_ne_bytes(*b"FLAS"),
        ];
        static PATTERNS: &[(&[u8], SaveType)] = &[
            (b"EEPROM_V", SaveType::EepromAuto),
            (b"SRAM_V", SaveType::Sram),
            (b"SRAM_F_V", SaveType::Sram),
            (b"FLASH_V", SaveType::Flash64K),
            (b"FLASH512_V", SaveType::Flash64K),
            (b"FLASH1M_V", SaveType::Flash128K),
        ];

        // First, do a high level search of each u32 word. If one of the
        // words is the prefix of one of the patterns, then do a full
        // comparison. This greatly improves the detection speed.
        for &prefix in PREFIXES {
            for (i, word) in data.chunks_exact(Self::STEP).enumerate() {
                if u32::from_ne_bytes([word[0], word[1], word[2], word[3]]) == prefix {
                    let region = &data[(i * 4)..];
                    for &(pattern, type_) in PATTERNS {
                        if region.starts_with(pattern) {
                            return Some(type_);
                        }
                    }
                }
            }
        }
        None
    }

    /// Process the next chunk of data.
    pub fn process(&mut self, data: &[u8]) {
        if self.detected.is_some() {
            return;
        }

        // Check the overlap of the last buffer to this buffer.
        let prefix = &data[..(data.len().min(Self::OVERLAP))];
        self.buffer.extend_from_slice(prefix);
        self.detected = Self::search(&self.buffer);
        if self.detected.is_some() {
            return;
        }

        self.detected = Self::search(data);
        let suffix = &data[(data.len().saturating_sub(Self::OVERLAP) & !(Self::STEP - 1))..];
        self.buffer.clear();
        self.buffer.extend_from_slice(suffix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_does_not_require_an_aligned_slice_address() {
        let data = b"xFLASH1M_V";
        assert_eq!(
            SaveTypeDetector::search(&data[1..]),
            Some(SaveType::Flash128K)
        );
    }

    #[test]
    fn detects_a_signature_split_across_chunks() {
        let mut detector = SaveTypeDetector::new();
        detector.process(b"xxxxFLAS");
        detector.process(b"H1M_Vxxx");
        assert_eq!(detector.get(), SaveType::Flash128K);
    }
}
