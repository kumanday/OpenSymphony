"""Temporary native diagnostic for atomic replacement under parent pins."""
import ctypes as c
from ctypes import wintypes as w
from pathlib import Path
import tempfile

k = c.WinDLL("kernel32", use_last_error=True)
k.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, c.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
k.CreateFileW.restype = w.HANDLE
k.CloseHandle.argtypes = [w.HANDLE]
k.SetFileAttributesW.argtypes = [w.LPCWSTR, w.DWORD]
k.MoveFileExW.argtypes = [w.LPCWSTR, w.LPCWSTR, w.DWORD]
k.SetFileInformationByHandle.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]

class Rename(c.Structure):
    _fields_ = [("replace", w.DWORD), ("root", w.HANDLE), ("length", w.DWORD), ("name", w.WCHAR * 260)]

class Status(c.Structure):
    _fields_ = [("status", c.c_void_p), ("information", c.c_size_t)]

nt = c.WinDLL("ntdll")
nt.NtSetInformationFile.argtypes = [w.HANDLE, c.POINTER(Status), c.c_void_p, w.DWORD, c.c_int]
nt.NtSetInformationFile.restype = w.LONG

with tempfile.TemporaryDirectory() as temp:
    root = Path(temp)
    target = root / "destination"
    target.write_text("original")
    pin = k.CreateFileW(str(root), 0x80000000, 1, None, 3, 0x02200000, None)
    assert pin != c.c_void_p(-1).value, c.get_last_error()
    try:
        for method in ["move", "win32-leaf", "nt-leaf"]:
            stage = root / method
            stage.write_text("replacement")
            attrs = k.SetFileAttributesW(str(stage), 0x80)
            print(method, "SetFileAttributesW", attrs, c.get_last_error(), flush=True)
            if method == "move":
                result = k.MoveFileExW(str(stage), str(target), 1)
                print(method, "rename", result, c.get_last_error(), flush=True)
            else:
                handle = k.CreateFileW(str(stage), 0xC0010000, 7, None, 3, 0x00200000, None)
                assert handle != c.c_void_p(-1).value, c.get_last_error()
                info = Rename(1, None, len("destination".encode("utf-16-le")), "destination")
                try:
                    if method == "win32-leaf":
                        result = k.SetFileInformationByHandle(handle, 3, c.byref(info), c.sizeof(info))
                        print(method, "rename", result, c.get_last_error(), flush=True)
                    else:
                        status = Status()
                        result = nt.NtSetInformationFile(handle, c.byref(status), c.byref(info), c.sizeof(info), 10)
                        print(method, "rename", hex(result & 0xffffffff), flush=True)
                finally:
                    k.CloseHandle(handle)
            print(method, "target", target.read_text(), "stage exists", stage.exists(), flush=True)
    finally:
        k.CloseHandle(pin)
