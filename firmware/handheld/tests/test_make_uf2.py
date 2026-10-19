import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location("make_uf2", Path(__file__).resolve().parents[1] / "make_uf2.py")
uf2 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(uf2)


class BundleTests(unittest.TestCase):
    def test_round_trip_and_partition_boundaries(self):
        image = bytes(range(256)) * 17 + b"last"
        system = bytes(range(256)) * (uf2.SYSTEM_DATA_SIZE // 256)
        commit = "0123456789abcdef" * 2 + "01234567"
        bundle = uf2.make_uf2(image, 0x100000, 4, 1, commit, system)
        segments = {"app": bytearray(), "system": bytearray()}
        addresses = set()
        for index, offset in enumerate(range(0, len(bundle), 512)):
            block = bundle[offset:offset + 512]
            m0, m1, flags, address, size, number, total, family = struct.unpack_from("<8I", block)
            self.assertEqual((m0, m1, flags, size, number, total, family),
                             (uf2.MAGIC_START0, uf2.MAGIC_START1, 0xa000, 256, index, len(bundle) // 512, uf2.FAMILY_ID_ESP32_S3))
            self.assertEqual(struct.unpack_from("<I", block, 508)[0], uf2.MAGIC_END)
            self.assertNotIn(address, addresses)
            addresses.add(address)
            segment = "app" if address < 0x600000 else "system"
            base = 0x100000 if segment == "app" else 0x600000
            self.assertEqual(address, base + len(segments[segment]))
            segments[segment].extend(block[32:288])
            tag_offset = 288
            tags = {}
            while block[tag_offset]:
                size = block[tag_offset]
                tag = int.from_bytes(block[tag_offset + 1:tag_offset + 4], "little")
                tags[tag] = block[tag_offset + 4:tag_offset + size]
                tag_offset += (size + 3) & ~3
            self.assertEqual(tags[uf2.TAG_GIT_COMMIT], commit.encode())
            self.assertEqual(tags[uf2.TAG_DEVICE_TYPE], b"bub!\0\0\4\1")
        self.assertEqual(segments["app"][:len(image)], image)
        self.assertEqual(set(segments["app"][len(image):]), {0})
        self.assertEqual(len(segments["app"]) % 4096, 0)
        self.assertEqual(segments["system"], system)
        self.assertEqual(max(addresses), 0x800000 - 256)

    def test_invalid_inputs_fail_before_writing(self):
        system = bytes(uf2.SYSTEM_DATA_SIZE)
        for image, base, commit, data in [
            (b"", 0x100000, "a" * 40, system),
            (b"x", 0, "a" * 40, system),
            (b"x", 0x100000, "bad", system),
            (b"x", 0x100000, "a" * 40, system[:-1]),
            (bytes(5 * 1024 * 1024 + 1), 0x100000, "a" * 40, system),
        ]:
            with self.subTest(base=base, length=len(image), commit=commit):
                with self.assertRaises(ValueError):
                    uf2.make_uf2(image, base, 4, 1, commit, data)


if __name__ == "__main__":
    unittest.main()
