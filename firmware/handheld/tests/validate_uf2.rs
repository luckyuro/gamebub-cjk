#[path = "../../handheld-dfu/src/uf2.rs"]
mod uf2;

fn main() {
    let mut args = std::env::args().skip(1);
    let commit = args.next().expect("expected source commit");
    for path in args {
        let data = std::fs::read(&path).unwrap();
        assert_eq!(data.len() % 512, 0);
        let mut previous = None;
        let mut system_blocks = 0;
        for (index, raw) in data.chunks_exact(512).enumerate() {
            #[repr(align(4))]
            struct Aligned([u8; 512]);
            let aligned = Aligned(raw.try_into().unwrap());
            let block = uf2::Uf2Block::from_bytes(&aligned.0).expect("DFU parser rejected block");
            assert_eq!(
                block.flags(),
                uf2::FLAG_FAMILY_ID_PRESENT | uf2::FLAG_EXTENSION_TAGS_PRESENT
            );
            assert_eq!(block.family_id(), 0xC47E5767);
            assert_eq!(block.block_number() as usize, index);
            assert_eq!(block.total_blocks() as usize, data.len() / 512);
            assert_eq!(block.payload().len(), 256);
            assert_eq!(
                block.get_extension_tag(0xC8A729),
                Some(b"bub!\0\0\x04\x01".as_slice())
            );
            assert_eq!(block.get_extension_tag(0x8A4E54), Some(commit.as_bytes()));
            let address = block.address();
            assert_eq!(address % 256, 0);
            assert!((0x100000..0x800000).contains(&address));
            if let Some(previous) = previous {
                assert!(address == previous + 256 || (previous < 0x600000 && address == 0x600000));
            } else {
                assert_eq!(address, 0x100000);
            }
            if address >= 0x600000 {
                system_blocks += 1;
            }
            previous = Some(address);
        }
        assert_eq!(system_blocks, 2 * 1024 * 1024 / 256);
        assert_eq!(previous, Some(0x800000 - 256));
        println!(
            "{path}: {} blocks accepted by production DFU parser",
            data.len() / 512
        );
    }
}
