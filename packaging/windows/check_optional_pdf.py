"""Release gate: optional system-PDF imports and Windows loader smoke, no DLL redistribution."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile


def imports(path):
    data = Path(path).read_bytes()

    def unpack(fmt, offset):
        size = struct.calcsize(fmt)
        if offset < 0 or offset + size > len(data):
            raise ValueError("Truncated PE")
        return struct.unpack_from(fmt, data, offset)

    if data[:2] != b"MZ":
        raise ValueError("Not a PE image")
    pe, = unpack("<I", 0x3C)
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE signature")
    machine, count, _, _, _, optional_size, _ = unpack("<HHIIIHH", pe + 4)
    optional = pe + 24
    magic, = unpack("<H", optional)
    if machine != 0x8664 or magic != 0x20B:
        raise ValueError("Expected x64 PE32+")
    if count > 96 or optional_size < 240:
        raise ValueError("Invalid PE headers")
    directories, = unpack("<I", optional + 108)
    if directories < 14:
        raise ValueError("Missing PE directories")
    sections = []
    for index in range(count):
        start = optional + optional_size + 40 * index
        virtual_size, address, raw_size, raw = unpack("<IIII", start + 8)
        sections.append((address, virtual_size, raw, raw_size))

    def rva_offset(rva):
        for address, virtual_size, raw, raw_size in sections:
            delta = rva - address
            if 0 <= delta < max(virtual_size, raw_size):
                if delta >= raw_size or raw + delta >= len(data):
                    raise ValueError("Unbacked PE RVA")
                return raw + delta
        raise ValueError("Unmapped PE RVA")

    def name(rva):
        offset = rva_offset(rva)
        end = data.find(b"\0", offset, min(offset + 512, len(data)))
        if end == -1:
            raise ValueError("Unterminated DLL name")
        return data[offset:end].decode("ascii").lower()

    def table(index, width, name_index):
        rva, size = unpack("<II", optional + 112 + 8 * index)
        if not rva:
            return set()
        if size > 1 << 20:
            raise ValueError("Oversized import directory")
        result = set()
        for offset in range(0, size, width):
            values = unpack("<" + "I" * (width // 4), rva_offset(rva + offset))
            if not any(values):
                return result
            if index == 13 and values[0] != 1:
                raise ValueError("Expected RVA-based delay descriptor")
            result.add(name(values[name_index]))
        raise ValueError("Unterminated import table")

    return table(1, 20, 3), table(13, 32, 1)


def verify(path):
    eager, delayed = imports(path)
    for required in ("windows.data.pdf.dll", "prntvpt.dll"):
        if required in eager or required not in delayed:
            raise ValueError(required + " must be delay-loaded, never a startup import")
    runtime = sorted(x for x in eager if x.startswith(("vcruntime", "msvcp", "api-ms-win-crt-")))
    if runtime:
        raise ValueError("Portable release still requires external CRT: " + ", ".join(runtime))
    print("PASS: x64 image; PDF renderer delay-loaded; no external VC++ CRT")


def smoke(path):
    if os.name != "nt":
        raise ValueError("Loader smoke requires Windows")
    import ctypes
    # Inherited only by our own child: suppress loader error dialogs in unattended CI.
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.SetErrorMode.argtypes = [ctypes.c_uint]
    kernel.SetErrorMode.restype = ctypes.c_uint
    previous = kernel.SetErrorMode(0x8003)
    try:
        with tempfile.TemporaryDirectory(prefix="craft-pdf-loader-") as temp:
            executable = Path(temp) / Path(path).name
            shutil.copy2(path, executable)
            for shadow in (False, True):
                if shadow:
                    (Path(temp) / "windows.data.pdf.dll").write_bytes(b"Synthetic invalid DLL: must not load at startup.")
                    (Path(temp) / "prntvpt.dll").write_bytes(b"Synthetic invalid DLL: must not load at startup.")
                result = subprocess.run([str(executable), "--version"], cwd=temp, timeout=30,
                                        capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
                if result.returncode:
                    raise ValueError(f"Loader smoke failed (shadow={shadow}, exit={result.returncode})")
                print(f"PASS: --version reaches application code (shadow DLL={shadow})")
    finally:
        kernel.SetErrorMode(previous)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    verify(args.binary)
    if args.smoke:
        smoke(args.binary)
