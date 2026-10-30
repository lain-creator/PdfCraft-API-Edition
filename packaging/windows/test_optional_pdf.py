"""Synthetic PE fixtures for the release import gate; no third-party binaries."""
from pathlib import Path
import struct
import tempfile
import unittest
from check_optional_pdf import imports, verify


def fixture(eager=(), delayed=("windows.data.pdf.dll", "prntvpt.dll")):
    data = bytearray(0x1200)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, 0x8664, 1, 0, 0, 0, 240, 0)
    optional = 0x98
    struct.pack_into("<H", data, optional, 0x20B)
    struct.pack_into("<I", data, optional + 108, 16)
    struct.pack_into("<IIII", data, optional + 240 + 8, 0x1000, 0x1000, 0x1000, 0x200)
    cursor = 0x700
    for index, names, offset, rva, width in ((1, eager, 0x300, 0x1100, 20), (13, delayed, 0x400, 0x1200, 32)):
        if not names:
            continue
        struct.pack_into("<II", data, optional + 112 + 8 * index, rva, width * (len(names) + 1))
        for number, name in enumerate(names):
            encoded = name.encode("ascii") + b"\0"
            data[cursor:cursor + len(encoded)] = encoded
            name_rva = cursor - 0x200 + 0x1000
            values = (0, 0, 0, name_rva, 0) if index == 1 else (1, name_rva, 0, 0, 0, 0, 0, 0)
            struct.pack_into("<" + "I" * len(values), data, offset + number * width, *values)
            cursor += len(encoded)
    return bytes(data)


class OptionalPdfTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.binary = Path(self.directory.name) / "Synthetic.exe"

    def write(self, data):
        self.binary.write_bytes(data)
        return self.binary

    def test_accepts_delay_only_renderer(self):
        verify(self.write(fixture()))

    def test_rejects_original_eager_renderer(self):
        with self.assertRaisesRegex(ValueError, "startup import"):
            verify(self.write(fixture(eager=("Windows.Data.Pdf.dll",), delayed=())))

    def test_rejects_both_eager_and_delay_renderer(self):
        with self.assertRaisesRegex(ValueError, "startup import"):
            verify(self.write(fixture(eager=("windows.data.pdf.dll",))))

    def test_rejects_external_crt(self):
        with self.assertRaisesRegex(ValueError, "external CRT"):
            verify(self.write(fixture(eager=("VCRUNTIME140.dll",))))

    def test_rejects_eager_print_ticket_component(self):
        with self.assertRaisesRegex(ValueError, "prntvpt.dll"):
            verify(self.write(fixture(eager=("prntvpt.dll",))))

    def test_rejects_no_renderer(self):
        with self.assertRaisesRegex(ValueError, "delay-loaded"):
            verify(self.write(fixture(delayed=())))

    def test_rejects_truncated_image(self):
        with self.assertRaises(ValueError):
            imports(self.write(fixture()[:300]))


if __name__ == "__main__":
    unittest.main()
