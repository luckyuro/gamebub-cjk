#!/usr/bin/env python3
"""Wrap a Game Bub ESP32-S3 application image in the UF2 format."""

import argparse
import hashlib
from pathlib import Path
import re
import struct
import subprocess


MAGIC_START0 = 0x0A324655
MAGIC_START1 = 0x9E5D5157
MAGIC_END = 0x0AB16F30
FLAG_FAMILY_ID_PRESENT = 0x00002000
FLAG_EXTENSION_TAGS_PRESENT = 0x00008000
FAMILY_ID_ESP32_S3 = 0xC47E5767
TAG_DEVICE_TYPE = 0xC8A729
TAG_GIT_COMMIT = 0x8A4E54
PAYLOAD_SIZE = 256
BLOCK_SIZE = 512


def extension_tag(tag_type: int, payload: bytes) -> bytes:
    size = 4 + len(payload)
    if size > 255:
        raise ValueError("UF2 extension tag is too large")
    tag = bytes([size]) + tag_type.to_bytes(3, "little") + payload
    return tag.ljust((len(tag) + 3) & ~3, b"\0")


def current_commit() -> str:
    repository = Path(__file__).resolve().parents[2]
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=repository, text=True
    ).strip()


def make_uf2(
    image: bytes,
    base_address: int,
    hardware_major: int,
    product: int,
    commit: str,
) -> bytes:
    if base_address % PAYLOAD_SIZE:
        raise ValueError("base address must be 256-byte aligned")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", commit):
        raise ValueError("commit must be a 40-character hexadecimal Git hash")

    padded_image = image.ljust(
        (len(image) + PAYLOAD_SIZE - 1) // PAYLOAD_SIZE * PAYLOAD_SIZE, b"\0"
    )
    total_blocks = len(padded_image) // PAYLOAD_SIZE
    device = b"bub!" + bytes([0, 0, hardware_major, product])
    extensions = (
        extension_tag(TAG_DEVICE_TYPE, device)
        + extension_tag(TAG_GIT_COMMIT, commit.lower().encode("ascii"))
        + b"\0\0\0\0"
    )
    if len(extensions) > BLOCK_SIZE - 32 - PAYLOAD_SIZE - 4:
        raise ValueError("UF2 extension tags do not fit in a block")

    flags = FLAG_FAMILY_ID_PRESENT | FLAG_EXTENSION_TAGS_PRESENT
    output = bytearray()
    for block_number in range(total_blocks):
        address = base_address + block_number * PAYLOAD_SIZE
        payload = padded_image[
            block_number * PAYLOAD_SIZE : (block_number + 1) * PAYLOAD_SIZE
        ]
        header = struct.pack(
            "<8I",
            MAGIC_START0,
            MAGIC_START1,
            flags,
            address,
            PAYLOAD_SIZE,
            block_number,
            total_blocks,
            FAMILY_ID_ESP32_S3,
        )
        block = header + payload + extensions
        block = block.ljust(BLOCK_SIZE - 4, b"\0") + struct.pack("<I", MAGIC_END)
        assert len(block) == BLOCK_SIZE
        output.extend(block)
    return bytes(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="ESP32 application .bin")
    parser.add_argument("output", type=Path, help="output .uf2")
    parser.add_argument("--base", type=lambda value: int(value, 0), default=0x100000)
    parser.add_argument(
        "--factory-size",
        type=lambda value: int(value, 0),
        default=5 * 1024 * 1024,
        help="refuse images that exceed this many bytes (default: 5 MiB)",
    )
    parser.add_argument("--hardware-major", type=int, default=4)
    parser.add_argument("--product", type=int, default=1)
    parser.add_argument("--commit", default=None)
    args = parser.parse_args()

    if not 0 <= args.hardware_major <= 255 or not 0 <= args.product <= 255:
        parser.error("hardware-major and product must fit in one byte")

    image = args.input.read_bytes()
    padded_size = (len(image) + PAYLOAD_SIZE - 1) // PAYLOAD_SIZE * PAYLOAD_SIZE
    if padded_size > args.factory_size:
        parser.error(
            f"image occupies {padded_size} bytes, exceeding the factory partition "
            f"by {padded_size - args.factory_size} bytes"
        )

    commit = args.commit or current_commit()
    uf2 = make_uf2(image, args.base, args.hardware_major, args.product, commit)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(uf2)
    print(
        f"wrote {args.output}: {len(uf2)} bytes, {len(uf2) // BLOCK_SIZE} blocks, "
        f"flash 0x{args.base:x}..0x{args.base + padded_size:x}, "
        f"sha256 {hashlib.sha256(uf2).hexdigest()}"
    )


if __name__ == "__main__":
    main()
