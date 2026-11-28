#!/usr/bin/env python3
"""Differential serial check: stock ESP32-S3 apps in Espressif QEMU vs orbe.

    scripts/diff-esp32s3.sh [--app NAME ...] [--list] [--cache DIR] [--orbe-run PATH]

Fetches the pinned Espressif QEMU release and Arduino-ESP32 3.3.12 bootloaders
into the cache directory when missing (each verified against its pinned
SHA-256), builds fail-loud user-space libSDL2/libslirp stubs if the system
lacks them, builds a merged flash image per app, runs each app in both headless
QEMU and `orbe-run`, normalizes the expected hardware-identity differences, and
exits non-zero with a unified diff on any mismatch, panic, watchdog reset, or
timeout.
"""

from __future__ import annotations

import argparse
import csv
import dataclasses
import difflib
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

QEMU_TAG = "esp-develop-9.2.2-20261109"
QEMU_RELEASE_BASE = f"https://github.com/espressif/qemu/releases/download/{QEMU_TAG}"
QEMU_ASSETS: dict[str, tuple[str, str]] = {
    "x86_64": (
        "qemu-xtensa-softmmu-esp_develop_9.2.2_20261109-x86_64-linux-gnu.tar.xz",
        "fbc3bffa9fa71192847969a627a584aabe2e86d85bed4902a096ff06c2b8a815",
    ),
    "aarch64": (
        "qemu-xtensa-softmmu-esp_develop_9.2.2_20261109-aarch64-linux-gnu.tar.xz",
        "dad4a8f44b5649b1f110963d479a77edb0ad2484cde79403e3a19c50efdd5281",
    ),
}
QEMU_BIN_SHA256: dict[str, str] = {
    "x86_64": "c15716f0bf981f046976b556c9856270b55b4e6a77ef00e2a124c2ef65b4932d",
}

BOOT_LIBS_URL = "https://github.com/espressif/arduino-esp32/releases/download/3.3.12/esp32s3-libs-3.3.12.zip"
BOOT_LIBS_SHA256 = "08f4e7292fee97dc011dc64e3c670a9a4e83a6b52a604c4b9b18ab2885cdd092"
BOOTLOADERS: dict[str, tuple[int, str, str]] = {
    # mode_name: (spi_mode_byte, elf_sha256, bin_sha256)
    "dio": (
        2,
        "d4c7788d72422224ea4b66b119b3d36bd20e33aa82e13eb826e75666c99ffbcd",
        "cacaf553a169634fed975073648158426db68668f6155d3264414649497c9114",
    ),
    "qio": (
        0,
        "92d91435aad61580c804fb5d3c42252415eec20f542c6c2fa1d3a974a666b686",
        "2bc0146d9d445f21b46d5f26c37e03655a29b7f2a61c96830c8cf164ccf9fd42",
    ),
}

ROM_SHA256 = "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd"
ROM_BYTES = 949552

FLASH_SIZE_NIBBLE_TO_MB: dict[int, int] = {0: 1, 1: 2, 2: 4, 3: 8, 4: 16}
FLASH_MB_TO_NIBBLE: dict[int, int] = {mb: nib for nib, mb in FLASH_SIZE_NIBBLE_TO_MB.items()}

PERIODIC_TOLERANCE = 0.25
DEFAULT_ORBE_TIMEOUT_S = 15.0
QEMU_CONNECT_TIMEOUT_S = 5.0
STDERR_TAIL_LINES = 20

# Fatal patterns that indicate a crash, panic, abort, watchdog reset, or unexpected reboot.
FATAL_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("Guru Meditation Error", re.compile(r"Guru Meditation Error")),
    ("abort()", re.compile(r"abort\(\) was called")),
    ("Backtrace", re.compile(r"^Backtrace:", re.MULTILINE)),
    ("Task watchdog", re.compile(r"Task watchdog got triggered")),
    ("Interrupt watchdog", re.compile(r"Interrupt wdt timeout")),
    ("Watchdog reset", re.compile(r"rst:0x[0-9a-f]+\s+\([^)]*WDT[^)]*\)", re.IGNORECASE)),
    ("Software/panic reset", re.compile(r"rst:0x[0-9a-f]+\s+\([^)]*(SW_CPU_RST|SW_SYS_RST|PANIC)[^)]*\)")),
    ("Boot partition error", re.compile(r"Failed to verify partition table|load partition table error")),
    ("Rebooting", re.compile(r"^Rebooting\.\.\.$", re.MULTILINE)),
)

IDF_TS_RE = re.compile(r"^([IWEVD]) \(\d+\) ")
CHIP_REV_LINE_RE = re.compile(r"^[IWEVD] \(<T>\) efuse_init: Chip rev:")
FLASH_CHIP_LINE_RE = re.compile(r"^[IWEVD] \(<T>\) spi_flash: detected chip:")
SILICON_REV_RE = re.compile(
    r"^(This is esp32s3 chip with \d+ CPU core\(s\), [^,]+, silicon revision )v\d+\.\d+"
    r"(, \d+MB (?:embedded|external) flash)$"
)
BOOT_RTCINIT_RE = re.compile(r"^W \(\d+\) rtcinit: o_code calibration fail$")


class OrbeTimeoutError(RuntimeError):
    """Raised when orbe-run exceeds its wall-clock timeout."""

    def __init__(self, timeout_s: float, stderr: str = "") -> None:
        self.timeout_s = timeout_s
        self.stderr = stderr
        msg = f"orbe-run timed out after {timeout_s:g} s"
        tail = _stderr_tail(stderr)
        if tail:
            msg += f"\nstderr:\n{tail}"
        super().__init__(msg)


def _stderr_tail(text: str, max_lines: int = STDERR_TAIL_LINES) -> str:
    lines = text.strip().splitlines()
    if not lines:
        return ""
    if len(lines) > max_lines:
        return "\n".join(["...", *lines[-max_lines:]])
    return "\n".join(lines)


def _terminate_process_group(proc: subprocess.Popen[bytes] | subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=0.5)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=2.0)
    except subprocess.TimeoutExpired:
        pass


@dataclasses.dataclass(frozen=True)
class PeriodicSpec:
    pattern: re.Pattern[str]
    period_ms: float
    min_ticks: int


@dataclasses.dataclass(frozen=True)
class AppSpec:
    name: str
    kind: str  # "fixture" or "circuit"
    rel_dir: str
    boot_mode: str  # "dio" or "qio"
    orbe_usart: str  # "UART0" or "USB_SERIAL_JTAG"
    orbe_stop_ms: int
    qemu_timeout_s: float
    qemu_quiet_s: float = 0.5
    orbe_timeout_s: float = DEFAULT_ORBE_TIMEOUT_S
    periodic: PeriodicSpec | None = None
    extra_pins: tuple[tuple[str, str], ...] = ()
    note: str = ""


APPS: tuple[AppSpec, ...] = (
    AppSpec(
        name="esp32s3-stock-serial",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-serial",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=650,
        qemu_timeout_s=5.0,
        periodic=PeriodicSpec(pattern=re.compile(r"^tick$"), period_ms=100.0, min_ticks=5),
    ),
    AppSpec(
        name="esp32s3-stock-blink",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-blink",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
    ),
    AppSpec(
        name="esp32s3-stock-crypto",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-crypto",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=600,
        qemu_timeout_s=8.0,
        qemu_quiet_s=0.8,
    ),
    AppSpec(
        name="esp32s3-stock-nvs",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-nvs",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=600,
        qemu_timeout_s=5.0,
        orbe_timeout_s=60.0,
    ),
    AppSpec(
        name="esp32s3-stock-eeprom",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-eeprom",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=600,
        qemu_timeout_s=5.0,
    ),
    AppSpec(
        name="esp32s3-stock-littlefs",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-littlefs",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=1000,
        qemu_timeout_s=10.0,
        qemu_quiet_s=1.2,
        orbe_timeout_s=30.0,
    ),
    AppSpec(
        name="esp32s3-stock-preferences",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-preferences",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=600,
        qemu_timeout_s=5.0,
        extra_pins=(("GPIO5", "gnd"),),
        note=(
            "GPIO5 tied to ground in orbe to match QEMU's unimplemented IO_MUX "
            "(INPUT_PULLUP reads LOW in QEMU), so both run setup() and press count=2"
        ),
    ),
    AppSpec(
        name="esp32s3-stock-analogwrite",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-analogwrite",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
        note="serial only; QEMU does not model LEDC electrical output",
    ),
    AppSpec(
        name="esp32s3-stock-fade",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-fade",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
        orbe_timeout_s=30.0,
        note="serial only; QEMU does not model LEDC electrical output",
    ),
    AppSpec(
        name="esp32s3-stock-servo",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-servo",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=950,
        qemu_timeout_s=6.0,
        periodic=PeriodicSpec(pattern=re.compile(r"^pulse (1000|1500|2000)$"), period_ms=200.0, min_ticks=4),
        note="serial only; QEMU does not model LEDC electrical output",
    ),
    AppSpec(
        name="esp32s3-stock-ledcdetach",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-ledcdetach",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
        note="serial only; QEMU does not model LEDC electrical output",
    ),
    AppSpec(
        name="esp32s3-stock-ledcreattach",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-ledcreattach",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
        note="serial only; QEMU does not model LEDC electrical output",
    ),
    AppSpec(
        name="esp32s3-stock-wire",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-wire",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
    ),
    AppSpec(
        name="esp32s3-stock-pcnt",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-pcnt",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=300,
        qemu_timeout_s=4.0,
        orbe_timeout_s=20.0,
        note="boot serial only; QEMU does not model PCNT pulse inputs",
    ),
    AppSpec(
        name="esp32s3-hello",
        kind="circuit",
        rel_dir="packages/engine/tests/circuits/esp32s3-hello",
        boot_mode="qio",
        orbe_usart="USB_SERIAL_JTAG",
        orbe_stop_ms=3500,
        qemu_timeout_s=8.0,
        periodic=PeriodicSpec(
            pattern=re.compile(r"^Restarting in \d+ seconds\.\.\.$"),
            period_ms=1000.0,
            min_ticks=3,
        ),
    ),
)

# Every remaining ESP32-S3 fixture, circuit, and DevKitC Block in the repo, with why QEMU cannot diff it.
EXCLUDED_APPS: dict[str, str] = {
    # Duplicate hello_world fixtures (identical app.bin and partitions.bin to circuits/esp32s3-hello)
    "packages/scene/tests/fixtures/esp32s3-devkitc": "duplicate of packages/engine/tests/circuits/esp32s3-hello",
    "packages/scene/tests/fixtures/esp32s3-hello": "duplicate of packages/engine/tests/circuits/esp32s3-hello",
    # Bare-metal ELF circuits (no flash/bootloader image and no serial output)
    "packages/engine/tests/circuits/esp32s3-blink": "bare-metal ELF without flash image or serial output",
    "packages/engine/tests/circuits/esp32s3-pattern": "bare-metal ELF driving SPI2 ST77916 panel with no serial output",
    # USB-Serial-JTAG CDC / interactive console apps
    "packages/scene/tests/fixtures/esp32s3-stock-cdc": (
        "USB-Serial-JTAG CDC (HWCDC); QEMU's misc.esp32c3.usb_serial_jtag has no chardev"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-cdc4": (
        "USB-Serial-JTAG CDC (HWCDC); QEMU's misc.esp32c3.usb_serial_jtag has no chardev"
    ),
    "packages/engine/tests/circuits/esp32s3-dualcore": (
        "IDF multicore app with CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y; in QEMU both cores reach "
        "FreeRTOS idle but UART0 stops after cpu_start: Multicore app"
    ),
    "packages/engine/tests/circuits/esp32s3-timers": (
        "waits for 'g' and writes via usb_serial_jtag_driver_install; QEMU's USB-Serial-JTAG stub has no RX/CDC"
    ),
    "packages/engine/tests/circuits/esp32s3-echo": (
        "tests USB-Serial-JTAG CDC RX/TX driver; QEMU's USB-Serial-JTAG stub has no RX/CDC"
    ),
    "packages/engine/tests/circuits/esp32s3-uart": (
        "waits for 'g' over USB-Serial-JTAG and loops UART0 through IO_MUX (unimplemented in QEMU)"
    ),
    "packages/engine/tests/circuits/esp32s3-pulls": (
        "waits for 'g' over USB-Serial-JTAG and tests IO_MUX pull-up/pull-down (unimplemented in QEMU)"
    ),
    "packages/engine/tests/circuits/esp32s3-encoder": (
        "waits for 'g' over USB-Serial-JTAG and reads external rotary encoder contacts"
    ),
    "packages/engine/tests/circuits/esp32s3-ring": (
        "waits for 'g' over USB-Serial-JTAG and drives RMT WS2812 ring (unimplemented in QEMU)"
    ),
    "packages/engine/tests/circuits/esp32s3-screen": (
        "waits for 'g' over USB-Serial-JTAG and drives external SPI2 ST77916 display"
    ),
    "packages/engine/tests/circuits/esp32s3-touch": (
        "waits for 'g' over USB-Serial-JTAG and reads external I2C0 CST816S touch controller"
    ),
    "packages/engine/tests/circuits/esp32s3-adc": (
        "waits for 'g' over USB-Serial-JTAG and reads SAR ADC (unmodeled in QEMU)"
    ),
    # SAR ADC and temperature sensor (unmodeled in QEMU)
    "packages/scene/tests/fixtures/esp32s3-stock-adc": "SAR ADC is unmodeled in QEMU",
    "packages/scene/tests/fixtures/esp32s3-stock-rtcio": "SAR ADC and GP-SPI2 are unmodeled in QEMU",
    "packages/scene/tests/fixtures/esp32s3-stock-temp": "internal temperature sensor is unmodeled in QEMU",
    # RMT (esp32s3.rmt is an unimplemented-device in QEMU; TX blocks forever)
    "packages/scene/tests/fixtures/esp32s3-stock-rgb": "RMT is unimplemented in QEMU (rgbLedWrite blocks)",
    "packages/scene/tests/fixtures/esp32s3-stock-neopixel": "RMT is unimplemented in QEMU (Adafruit_NeoPixel blocks)",
    "packages/scene/tests/fixtures/esp32s3-stock-rmtstrip": "RMT is unimplemented in QEMU (rmtWrite blocks)",
    "blocks/devkitc-button-led": "calls rgbLedWrite (RMT is unimplemented in QEMU) and reads button on INPUT_PULLUP",
    # LEDC features unmodeled in QEMU (registers read 0, causing abort() or invalid-duty errors)
    "packages/scene/tests/fixtures/esp32s3-stock-tone": "QEMU lacks LEDC registers; Arduino tone() calls abort()",
    "packages/scene/tests/fixtures/esp32s3-stock-tonefast": "QEMU lacks LEDC registers; Arduino tone() calls abort()",
    "packages/scene/tests/fixtures/esp32s3-stock-tonemelody": (
        "QEMU lacks LEDC registers; Arduino tone() calls abort()"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-tonerestart": (
        "QEMU lacks LEDC registers; Arduino tone() calls abort()"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-melody": "QEMU lacks LEDC registers; Arduino tone() calls abort()",
    "packages/scene/tests/fixtures/esp32s3-stock-ledcfade": (
        "QEMU lacks LEDC registers; ledc_set_fade_time_and_start fails with invalid target_duty"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-ledcfadedetach": (
        "QEMU lacks LEDC registers; ledc_set_fade_time_and_start fails with invalid target_duty"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-ledcfreq": (
        "QEMU lacks LEDC registers; ledcChangeFrequency() calls abort()"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-ledcres": (
        "QEMU lacks LEDC registers; ledcChangeFrequency() calls abort()"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-ledcshared": (
        "QEMU lacks LEDC registers; ledcChangeFrequency() calls abort()"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-ledctone": (
        "QEMU lacks LEDC registers; ledcWriteTone() calls abort()"
    ),
    # External I2C / GP-SPI devices and bus pull-ups
    "packages/scene/tests/fixtures/esp32s3-stock-oled": "requires external SSD1306 OLED on I2C0 (unmodeled in QEMU)",
    "packages/scene/tests/fixtures/esp32s3-stock-wire1": (
        "requires external BMP280 on I2C0 and SSD1306 on I2C1 (unmodeled in QEMU)"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-wirelong": (
        "requires external SSD1306 OLED on I2C0 (unmodeled in QEMU)"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-wirenack": (
        "I2C controller and bus pull-ups are unmodeled in QEMU (scan returns timeout 5 instead of NACK 2)"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-wirenackother": (
        "requires external SSD1306 OLED on I2C0 (unmodeled in QEMU)"
    ),
    "packages/scene/tests/fixtures/esp32s3-stock-hspi": (
        "GP-SPI2/SPI3 are unmodeled in QEMU (MISO reads 0x00 instead of idle-high 0xff)"
    ),
    "blocks/devkitc-gy-bmp280": "requires external GY-BMP280 sensor on I2C0 (unmodeled in QEMU)",
    # External GPIO input / IO_MUX pull-ups
    "packages/scene/tests/fixtures/esp32s3-stock-button": (
        "IO_MUX is unimplemented in QEMU; INPUT_PULLUP reads LOW ('down' instead of 'open')"
    ),
    # Wi-Fi radio / network
    "packages/scene/tests/fixtures/esp32s3-stock-station": "Wi-Fi radio and AP are unmodeled in QEMU (-nic none)",
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while chunk := fh.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def download_verified(url: str, dest: Path, expected_sha256: str) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    if tmp.exists():
        tmp.unlink()
    req = urllib.request.Request(url, headers={"User-Agent": "orbe-diff-esp32s3"})
    with urllib.request.urlopen(req, timeout=180) as resp, tmp.open("wb") as out:
        shutil.copyfileobj(resp, out)
    got = sha256_file(tmp)
    if got != expected_sha256:
        tmp.unlink(missing_ok=True)
        raise RuntimeError(f"SHA-256 mismatch for {url}: got {got}, expected {expected_sha256}")
    tmp.replace(dest)


def elf_to_esp32s3_bin(elf_bytes: bytes, flash_mode: int, flash_size_nibble: int = 2, flash_freq: int = 0xF) -> bytes:
    """Convert an ESP32-S3 bootloader ELF into an ESP image binary (matches esptool elf2image)."""
    if elf_bytes[:4] != b"\x7fELF" or elf_bytes[4] != 1 or elf_bytes[5] != 1:
        raise ValueError("not a 32-bit little-endian ELF")
    e_entry, _e_phoff, e_shoff, _, _, _e_phentsize, _e_phnum, e_shentsize, e_shnum, _ = struct.unpack_from(
        "<IIIIHHHHHH", elf_bytes, 24
    )
    raw_segs: list[tuple[int, bytes]] = []
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        _sh_name, sh_type, sh_flags, sh_addr, sh_offset, sh_size = struct.unpack_from("<IIIIII", elf_bytes, off)
        if sh_type == 1 and (sh_flags & 2) and sh_size > 0 and sh_addr != 0:
            raw_segs.append((sh_addr, elf_bytes[sh_offset : sh_offset + sh_size]))
    raw_segs.sort(key=lambda s: s[0])
    segs: list[tuple[int, bytes]] = []
    for addr, data in raw_segs:
        if segs and addr == segs[-1][0] + len(segs[-1][1]):
            segs[-1] = (segs[-1][0], segs[-1][1] + data)
        else:
            segs.append((addr, data))

    out = bytearray()
    out += struct.pack("<BBBBI", 0xE9, len(segs), flash_mode, (flash_size_nibble << 4) | flash_freq, e_entry)
    out += struct.pack("<BBBBHBHH4sB", 0xEE, 0, 0, 0, 9, 0, 0, 0xFFFF, b"\x00" * 4, 1)
    checksum = 0xEF
    for addr, data in segs:
        if len(data) % 4:
            data = data + b"\x00" * (4 - len(data) % 4)
        out += struct.pack("<II", addr, len(data))
        out += data
        for b in data:
            checksum ^= b
    pad_len = (16 - ((len(out) + 1) % 16)) % 16
    out += b"\x00" * pad_len
    out.append(checksum)
    out += hashlib.sha256(out).digest()
    return bytes(out)


def patch_bootloader_flash_size(boot_bytes: bytes, flash_mb: int) -> bytes:
    """Update the flash-size nibble in an ESP32-S3 bootloader image and recompute its SHA-256."""
    nibble = FLASH_MB_TO_NIBBLE[flash_mb]
    b = bytearray(boot_bytes)
    has_hash = len(b) > 32 and b[23] == 1
    body_end = len(b) - 32 if has_hash else len(b)
    b[3] = (nibble << 4) | (b[3] & 0x0F)
    if has_hash:
        b[body_end:] = hashlib.sha256(b[:body_end]).digest()
    return bytes(b)


def ensure_qemu(cache_dir: Path) -> Path:
    if sys.platform != "linux":
        raise RuntimeError(f"Espressif QEMU differential check is Linux-only (got {sys.platform})")
    arch = platform.machine()
    if arch not in QEMU_ASSETS:
        raise RuntimeError(f"unsupported architecture for pinned Espressif QEMU: {arch}")
    qemu_bin = cache_dir / "qemu-esp32s3" / "qemu" / "bin" / "qemu-system-xtensa"
    expected_bin_sha = QEMU_BIN_SHA256.get(arch)
    if qemu_bin.is_file():
        if expected_bin_sha is None or sha256_file(qemu_bin) == expected_bin_sha:
            return qemu_bin
        print(f"diff-esp32s3: cached {qemu_bin} hash mismatch; re-fetching", file=sys.stderr)

    asset_name, asset_sha = QEMU_ASSETS[arch]
    url = f"{QEMU_RELEASE_BASE}/{asset_name}"
    print(f"diff-esp32s3: fetching {asset_name}...", file=sys.stderr)
    with tempfile.TemporaryDirectory(prefix="orbe-qemu-fetch-") as td:
        archive = Path(td) / asset_name
        download_verified(url, archive, asset_sha)
        dest = cache_dir / "qemu-esp32s3"
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive, "r:xz") as tf:
            tf.extractall(dest, filter="data")
    if not qemu_bin.is_file():
        raise RuntimeError(f"extracted archive did not contain {qemu_bin}")
    if expected_bin_sha is not None and sha256_file(qemu_bin) != expected_bin_sha:
        raise RuntimeError(f"{qemu_bin} SHA-256 mismatch after extract")
    return qemu_bin


def ensure_bootloaders(cache_dir: Path) -> dict[str, Path]:
    boot_dir = cache_dir / "esp32s3-boot"
    boot_dir.mkdir(parents=True, exist_ok=True)
    paths: dict[str, Path] = {}
    missing: list[str] = []
    for mode, (_mode_byte, _elf_sha, bin_sha) in BOOTLOADERS.items():
        p = boot_dir / f"bootloader_{mode}_80m.bin"
        paths[mode] = p
        if not p.is_file() or sha256_file(p) != bin_sha:
            missing.append(mode)
    if not missing:
        return paths

    # Check if valid ELFs are already in cache or local Arduino install before downloading the zip.
    arduino_bin = Path.home() / ".arduino15" / "packages" / "esp32" / "tools" / "esp32s3-libs" / "3.3.12" / "bin"
    need_zip = False
    elf_paths: dict[str, Path] = {}
    for mode in missing:
        _mode_byte, elf_sha, _bin_sha = BOOTLOADERS[mode]
        for candidate in (boot_dir / f"bootloader_{mode}_80m.elf", arduino_bin / f"bootloader_{mode}_80m.elf"):
            if candidate.is_file() and sha256_file(candidate) == elf_sha:
                elf_paths[mode] = candidate
                break
        else:
            need_zip = True

    if need_zip:
        print("diff-esp32s3: fetching Arduino-ESP32 3.3.12 esp32s3-libs bootloaders...", file=sys.stderr)
        with tempfile.TemporaryDirectory(prefix="orbe-boot-fetch-") as td:
            zip_path = Path(td) / "esp32s3-libs-3.3.12.zip"
            download_verified(BOOT_LIBS_URL, zip_path, BOOT_LIBS_SHA256)
            with zipfile.ZipFile(zip_path) as zf:
                names = zf.namelist()
                for mode in missing:
                    if mode in elf_paths:
                        continue
                    suffix = f"bin/bootloader_{mode}_80m.elf"
                    matches = [n for n in names if n.endswith(suffix)]
                    if not matches:
                        raise RuntimeError(f"esp32s3-libs-3.3.12.zip does not contain {suffix}")
                    data = zf.read(matches[0])
                    _mode_byte, elf_sha, _bin_sha = BOOTLOADERS[mode]
                    got = hashlib.sha256(data).hexdigest()
                    if got != elf_sha:
                        raise RuntimeError(f"{matches[0]} SHA-256 mismatch: got {got}, expected {elf_sha}")
                    elf_dest = boot_dir / f"bootloader_{mode}_80m.elf"
                    elf_dest.write_bytes(data)
                    elf_paths[mode] = elf_dest

    for mode in missing:
        mode_byte, _elf_sha, bin_sha = BOOTLOADERS[mode]
        bin_bytes = elf_to_esp32s3_bin(elf_paths[mode].read_bytes(), flash_mode=mode_byte)
        got = hashlib.sha256(bin_bytes).hexdigest()
        if got != bin_sha:
            raise RuntimeError(f"generated bootloader_{mode}_80m.bin SHA-256 mismatch: got {got}, expected {bin_sha}")
        paths[mode].write_bytes(bin_bytes)
    return paths


def ensure_userspace_libs(qemu_bin: Path, cache_dir: Path) -> Path:
    """Ensure libSDL2-2.0.so.0 and libslirp.so.0 resolve for headless QEMU without system installs."""
    lib_dir = cache_dir / "prefix" / "lib"
    lib_dir.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    existing_ld = env.get("LD_LIBRARY_PATH", "")
    env["LD_LIBRARY_PATH"] = f"{lib_dir}:{existing_ld}" if existing_ld else str(lib_dir)

    ldd_out = subprocess.check_output(["ldd", str(qemu_bin)], env=env, text=True)
    missing_libs = {
        line.split("=>")[0].strip() for line in ldd_out.splitlines() if "=> not found" in line and "=>" in line
    }
    if not missing_libs:
        return lib_dir

    stubbable = {"libSDL2-2.0.so.0", "libslirp.so.0"}
    unsupported = missing_libs - stubbable
    if unsupported:
        raise RuntimeError(f"qemu-system-xtensa is missing shared libraries: {', '.join(sorted(unsupported))}")

    cc = shutil.which("cc") or shutil.which("gcc")
    if not cc:
        raise RuntimeError("cc is required to build user-space fail-loud stubs for " + ", ".join(sorted(missing_libs)))
    nm_out = subprocess.check_output(["nm", "-u", str(qemu_bin)], text=True)

    with tempfile.TemporaryDirectory(prefix="orbe-qemu-stubs-") as td:
        tmp = Path(td)
        if "libSDL2-2.0.so.0" in missing_libs:
            sdl_syms = sorted({line.split()[-1].split("@")[0] for line in nm_out.splitlines() if " U SDL_" in line})
            c_path = tmp / "sdl_stub.c"
            lines = ["#include <stdio.h>", "#include <stdlib.h>"]
            for sym in sdl_syms:
                lines.append(
                    f'void {sym}(void) {{ fprintf(stderr, "diff-esp32s3: unexpected call to {sym}\\n"); abort(); }}'
                )
            c_path.write_text("\n".join(lines) + "\n")
            subprocess.check_call(
                [
                    cc,
                    "-shared",
                    "-fPIC",
                    "-Wl,-soname,libSDL2-2.0.so.0",
                    "-o",
                    str(lib_dir / "libSDL2-2.0.so.0"),
                    str(c_path),
                ]
            )

        if "libslirp.so.0" in missing_libs:
            slirp_syms = sorted({line.split()[-1] for line in nm_out.splitlines() if " U slirp_" in line})
            c_path = tmp / "slirp_stub.c"
            map_path = tmp / "slirp.map"
            by_ver: dict[str, list[str]] = {}
            lines = ["#include <stdio.h>", "#include <stdlib.h>"]
            for sym in slirp_syms:
                name, ver = sym.split("@", 1) if "@" in sym else (sym, "SLIRP_4.0")
                by_ver.setdefault(ver, []).append(name)
                lines.append(
                    f'void {name}(void) {{ fprintf(stderr, "diff-esp32s3: unexpected call to {name}\\n"); abort(); }}'
                )
            c_path.write_text("\n".join(lines) + "\n")
            map_lines: list[str] = []
            prev_ver = ""
            for ver in sorted(by_ver):
                map_lines.append(f"{ver} {{")
                map_lines.append("  global:")
                for name in sorted(by_ver[ver]):
                    map_lines.append(f"    {name};")
                if not prev_ver:
                    map_lines.append("  local: *;")
                map_lines.append(f"}} {prev_ver};" if prev_ver else "};")
                prev_ver = ver
            map_path.write_text("\n".join(map_lines) + "\n")
            subprocess.check_call(
                [
                    cc,
                    "-shared",
                    "-fPIC",
                    "-Wl,-soname,libslirp.so.0",
                    f"-Wl,--version-script={map_path}",
                    "-o",
                    str(lib_dir / "libslirp.so.0"),
                    str(c_path),
                ]
            )

    ldd_after = subprocess.check_output(["ldd", str(qemu_bin)], env=env, text=True)
    if "not found" in ldd_after:
        raise RuntimeError(f"qemu-system-xtensa still has unresolved libraries:\n{ldd_after}")
    return lib_dir


def resolve_app_images(app: AppSpec) -> tuple[Path, Path]:
    base = ROOT / app.rel_dir
    if app.kind == "fixture":
        part = base / "partitions.bin"
        bin_path = base / "app.bin"
        if not part.is_file() or not bin_path.is_file():
            raise FileNotFoundError(f"missing fixture binaries under {base}")
        return part, bin_path
    if app.kind == "circuit":
        nl = json.loads((base / "netlist.json").read_text())
        chip = next(p for p in nl["parts"] if p["kind"] == "esp32s3")
        regions = {r["offset"]: base / "blobs" / r["sha256"] for r in chip["firmware"]["flash"]}
        return regions[0x8000], regions[0x10000]
    raise ValueError(f"unknown app kind {app.kind}")


def build_merged_flash(boot_path: Path, part_path: Path, app_path: Path, dest: Path) -> int:
    app_bytes = app_path.read_bytes()
    part_bytes = part_path.read_bytes()
    if len(app_bytes) < 24 or app_bytes[0] != 0xE9:
        raise ValueError(f"{app_path} is not a valid ESP32-S3 app image")
    size_nibble = (app_bytes[3] >> 4) & 0xF
    flash_mb = FLASH_SIZE_NIBBLE_TO_MB.get(size_nibble, 4)
    boot_bytes = patch_bootloader_flash_size(boot_path.read_bytes(), flash_mb)

    image = bytearray(b"\xff" * (flash_mb * 1024 * 1024))
    image[0 : len(boot_bytes)] = boot_bytes
    image[0x8000 : 0x8000 + len(part_bytes)] = part_bytes
    image[0x10000 : 0x10000 + len(app_bytes)] = app_bytes
    dest.write_bytes(image)
    return flash_mb


def run_qemu(
    qemu_bin: Path,
    lib_dir: Path,
    flash_img: Path,
    app: AppSpec,
) -> tuple[list[tuple[float, str]], str, int | None]:
    """Run headless QEMU and return timestamped serial lines, stderr, and early exit code (if any)."""
    env = os.environ.copy()
    existing_ld = env.get("LD_LIBRARY_PATH", "")
    env["LD_LIBRARY_PATH"] = f"{lib_dir}:{existing_ld}" if existing_ld else str(lib_dir)

    with tempfile.TemporaryDirectory(prefix="orbe-qemu-run-") as td:
        sock_path = Path(td) / "serial.sock"
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as srv:
            srv.bind(str(sock_path))
            srv.listen(1)
            srv.settimeout(0.05)

            cmd = [
                str(qemu_bin),
                "-nographic",
                "-display",
                "none",
                "-machine",
                "esp32s3",
                "-drive",
                f"file={flash_img},if=mtd,format=raw",
                "-nic",
                "none",
                "-monitor",
                "none",
                "-serial",
                f"unix:{sock_path}",
            ]
            proc = subprocess.Popen(
                cmd,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
            lines: list[tuple[float, str]] = []
            partial = bytearray()
            early_rc: int | None = None
            saw_entry = False
            app_line_count = 0
            periodic_count = 0
            last_app_line_t: float | None = None
            connect_deadline = time.monotonic() + min(QEMU_CONNECT_TIMEOUT_S, app.qemu_timeout_s)

            try:
                conn: socket.socket | None = None
                while conn is None:
                    rc = proc.poll()
                    if rc is not None:
                        early_rc = rc
                        break
                    try:
                        conn, _ = srv.accept()
                    except TimeoutError:
                        rc = proc.poll()
                        if rc is not None:
                            early_rc = rc
                            break
                        if time.monotonic() >= connect_deadline:
                            break

                if conn is not None:
                    try:
                        conn.settimeout(0.05)
                        start = time.monotonic()
                        while True:
                            now = time.monotonic()
                            elapsed = now - start
                            if elapsed >= app.qemu_timeout_s:
                                break
                            if app.periodic is not None:
                                if periodic_count >= app.periodic.min_ticks:
                                    break
                            elif (
                                app_line_count > 0
                                and last_app_line_t is not None
                                and (now - last_app_line_t) >= app.qemu_quiet_s
                            ):
                                break
                            try:
                                chunk = conn.recv(4096)
                                if not chunk:
                                    try:
                                        early_rc = proc.wait(timeout=0.2)
                                    except subprocess.TimeoutExpired:
                                        pass
                                    break
                                t_rel = time.monotonic() - start
                                for b in chunk:
                                    if b == 0x0A:
                                        line = partial.decode("utf-8", errors="replace").rstrip("\r")
                                        lines.append((t_rel, line))
                                        partial.clear()
                                        if line.startswith("entry 0x"):
                                            saw_entry = True
                                        elif saw_entry:
                                            if not BOOT_RTCINIT_RE.match(line) and line != "":
                                                app_line_count += 1
                                                last_app_line_t = time.monotonic()
                                            if app.periodic is not None and app.periodic.pattern.match(line):
                                                periodic_count += 1
                                    else:
                                        partial.append(b)
                            except TimeoutError:
                                rc = proc.poll()
                                if rc is not None:
                                    early_rc = rc
                                    break
                        if partial:
                            lines.append(
                                (time.monotonic() - start, partial.decode("utf-8", errors="replace").rstrip("\r"))
                            )
                    finally:
                        conn.close()
            finally:
                if proc.poll() is None:
                    _terminate_process_group(proc)
                    try:
                        _out, err_bytes = proc.communicate(timeout=2.0)
                    except subprocess.TimeoutExpired:
                        _out, err_bytes = b"", b""
                else:
                    early_rc = proc.returncode
                    _out, err_bytes = proc.communicate()

    return lines, err_bytes.decode("utf-8", errors="replace"), early_rc


def run_orbe(
    orbe_run: Path,
    rom_cache: Path,
    part_path: Path,
    app_path: Path,
    app: AppSpec,
    *,
    orbe_timeout_s: float | None = None,
) -> tuple[list[tuple[float, str]], dict]:
    """Run orbe-run on a minimal ESP32-S3 netlist and return timestamped lines and the JSON report."""
    part_bytes = part_path.read_bytes()
    app_bytes = app_path.read_bytes()
    part_sha = hashlib.sha256(part_bytes).hexdigest()
    app_sha = hashlib.sha256(app_bytes).hexdigest()

    pins: dict[str, str] = {"VCC": "vcc", "GND": "gnd"}
    for pin, net in app.extra_pins:
        pins[pin] = net

    with tempfile.TemporaryDirectory(prefix="orbe-diff-run-") as td:
        tmp = Path(td)
        bdir = tmp / "blobs"
        bdir.mkdir()
        (bdir / part_sha).write_bytes(part_bytes)
        (bdir / app_sha).write_bytes(app_bytes)

        netlist = {
            "version": 1,
            "seed": 0,
            "analysis": {
                "transient": {
                    "stop_ps": int(app.orbe_stop_ms * 1_000_000_000),
                    "max_step_ps": 100_000_000_000,
                    "grid_ps": 100_000_000_000,
                }
            },
            "ground": "gnd",
            "nets": ["gnd", "vcc"],
            "parts": [
                {
                    "name": "v1",
                    "kind": "voltage_source",
                    "pins": {"p": "vcc", "n": "gnd"},
                    "voltage": {"value": 3.3, "unit": "V"},
                },
                {
                    "name": "u1",
                    "kind": "esp32s3",
                    "pins": pins,
                    "rom": {"sha256": ROM_SHA256, "bytes": ROM_BYTES},
                    "firmware": {
                        "flash": [
                            {"offset": 32768, "sha256": part_sha, "bytes": len(part_bytes)},
                            {"offset": 65536, "sha256": app_sha, "bytes": len(app_bytes)},
                        ]
                    },
                    "output": {
                        "high_drop": {"value": 0.0, "unit": "V"},
                        "high_resistance": {"value": 16.5, "unit": "ohm"},
                        "low_drop": {"value": 0.0, "unit": "V"},
                        "low_resistance": {"value": 17.68, "unit": "ohm"},
                    },
                    "pull_up_resistance": {"value": 45000.0, "unit": "ohm"},
                    "leakage_resistance": {"value": 66000000.0, "unit": "ohm"},
                    "input_threshold": {
                        "low": {"value": 0.25, "unit": "1"},
                        "high": {"value": 0.75, "unit": "1"},
                    },
                },
            ],
        }
        nl_path = tmp / "netlist.json"
        out_dir = tmp / "run"
        nl_path.write_text(json.dumps(netlist))

        timeout_s = app.orbe_timeout_s if orbe_timeout_s is None else orbe_timeout_s
        proc = subprocess.Popen(
            [
                str(orbe_run),
                "run",
                str(nl_path),
                "--out",
                str(out_dir),
                "--blobs",
                str(rom_cache),
                "--blobs",
                str(bdir),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        try:
            stdout, stderr = proc.communicate(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            _terminate_process_group(proc)
            try:
                _stdout, stderr = proc.communicate(timeout=1.0)
            except subprocess.TimeoutExpired:
                stderr = ""
            raise OrbeTimeoutError(timeout_s, stderr) from None

        try:
            report = json.loads(stdout) if stdout.strip() else {"status": "error", "stderr": stderr}
        except json.JSONDecodeError:
            report = {"status": "error", "stdout": stdout, "stderr": stderr}
        if proc.returncode != 0 or report.get("errors", 0) != 0:
            raise RuntimeError(f"orbe-run failed (rc={proc.returncode}): {json.dumps(report, indent=2)}")

        lines: list[tuple[float, str]] = []
        serial_csv = out_dir / "traces" / "serial.csv"
        if serial_csv.is_file():
            partial = bytearray()
            last_t_s = 0.0
            with serial_csv.open(newline="") as fh:
                for row in csv.DictReader(fh):
                    if row["usart"] != app.orbe_usart:
                        continue
                    t_s = int(row["t_ps"]) / 1e12
                    b = int(row["byte"])
                    last_t_s = t_s
                    if b == 0x0A:
                        lines.append((t_s, partial.decode("utf-8", errors="replace").rstrip("\r")))
                        partial.clear()
                    else:
                        partial.append(b)
            if partial:
                lines.append((last_t_s, partial.decode("utf-8", errors="replace").rstrip("\r")))
        return lines, report


def detect_fatal(raw_lines: list[tuple[float, str]], *, is_qemu: bool) -> str | None:
    """Return a failure reason if serial output shows a panic, abort, watchdog reset, or reboot."""
    texts = [line for _, line in raw_lines]
    if is_qemu:
        rom_banners = [i for i, line in enumerate(texts) if line.startswith("ESP-ROM:")]
        if len(rom_banners) > 1:
            return f"unexpected second ROM banner / reboot at line {rom_banners[1] + 1}"
    joined = "\n".join(texts)
    for label, pat in FATAL_PATTERNS:
        m = pat.search(joined)
        if m:
            return f"{label}: {m.group(0)!r}"
    return None


def strip_qemu_boot_banner(raw_lines: list[tuple[float, str]]) -> list[tuple[float, str]]:
    """Strip the ESP32-S3 ROM and second-stage bootloader banner from QEMU serial output."""
    start = 0
    for i, (_t, line) in enumerate(raw_lines):
        if line.startswith("entry 0x"):
            start = i + 1
            break
    rest = raw_lines[start:]
    # Strip the second-stage bootloader's rtcinit warning + blank line before app startup.
    if rest and BOOT_RTCINIT_RE.match(rest[0][1]):
        rest = rest[1:]
        if rest and rest[0][1] == "":
            rest = rest[1:]
    return rest


def normalize_lines(lines: list[tuple[float, str]], *, is_qemu: bool) -> list[tuple[float, str]]:
    """Normalize serial output: strip ROM/bootloader banner, IDF timestamps, Chip rev, and flash chip."""
    working = strip_qemu_boot_banner(lines) if is_qemu else list(lines)
    out: list[tuple[float, str]] = []
    for t_s, raw in working:
        line = IDF_TS_RE.sub(r"\1 (<T>) ", raw)
        if CHIP_REV_LINE_RE.match(line):
            continue
        if FLASH_CHIP_LINE_RE.match(line):
            continue
        line = SILICON_REV_RE.sub(r"\1v<REV>\2", line)
        out.append((t_s, line))
    return out


def split_periodic(
    lines: list[tuple[float, str]],
    periodic: PeriodicSpec,
) -> tuple[list[tuple[float, str]], list[tuple[float, str]], list[tuple[float, str]]]:
    """Split normalized lines into (prefix, periodic_ticks, trailing_non_periodic)."""
    prefix: list[tuple[float, str]] = []
    ticks: list[tuple[float, str]] = []
    trailing: list[tuple[float, str]] = []
    in_ticks = False
    for item in lines:
        if periodic.pattern.match(item[1]):
            in_ticks = True
            ticks.append(item)
        elif not in_ticks:
            prefix.append(item)
        else:
            trailing.append(item)
    return prefix, ticks, trailing


def check_periodic_rate(
    label: str,
    ticks: list[tuple[float, str]],
    periodic: PeriodicSpec,
) -> tuple[float | None, str | None]:
    if len(ticks) < periodic.min_ticks:
        return None, f"{label}: got {len(ticks)} periodic ticks, need at least {periodic.min_ticks}"
    deltas_ms = [(ticks[i][0] - ticks[i - 1][0]) * 1000.0 for i in range(1, len(ticks))]
    mean_ms = sum(deltas_ms) / len(deltas_ms)
    lo = periodic.period_ms * (1.0 - PERIODIC_TOLERANCE)
    hi = periodic.period_ms * (1.0 + PERIODIC_TOLERANCE)
    if not (lo <= mean_ms <= hi):
        return mean_ms, (
            f"{label}: mean period {mean_ms:.1f} ms outside ±{int(PERIODIC_TOLERANCE * 100)}% of "
            f"{periodic.period_ms:.1f} ms ({lo:.1f}..{hi:.1f} ms); deltas={[round(d, 1) for d in deltas_ms]}"
        )
    return mean_ms, None


def compare_outputs(
    app: AppSpec,
    qemu_raw: list[tuple[float, str]],
    orbe_raw: list[tuple[float, str]],
) -> tuple[bool, str]:
    """Compare raw QEMU and orbe outputs for one app. Returns (ok, summary_or_error)."""
    if fatal := detect_fatal(qemu_raw, is_qemu=True):
        return False, f"QEMU fatal output: {fatal}"
    if fatal := detect_fatal(orbe_raw, is_qemu=False):
        return False, f"orbe fatal output: {fatal}"

    qemu_norm = normalize_lines(qemu_raw, is_qemu=True)
    orbe_norm = normalize_lines(orbe_raw, is_qemu=False)

    if not qemu_norm:
        return False, f"QEMU timed out or produced no app serial output within {app.qemu_timeout_s:.1f}s"
    if not orbe_norm:
        return False, f"orbe produced no serial output on {app.orbe_usart} within {app.orbe_stop_ms}ms"

    if app.periodic is None:
        q_texts = [line for _, line in qemu_norm]
        o_texts = [line for _, line in orbe_norm]
        if q_texts != o_texts:
            diff = "\n".join(
                difflib.unified_diff(
                    q_texts,
                    o_texts,
                    fromfile=f"qemu/{app.name}",
                    tofile=f"orbe/{app.name}",
                    lineterm="",
                )
            )
            return False, f"serial mismatch:\n{diff}"
        return True, f"{len(q_texts)} line(s) match"

    q_pre, q_ticks, q_trail = split_periodic(qemu_norm, app.periodic)
    o_pre, o_ticks, o_trail = split_periodic(orbe_norm, app.periodic)

    if q_trail or o_trail:
        diff = "\n".join(
            difflib.unified_diff(
                [line for _, line in qemu_norm],
                [line for _, line in orbe_norm],
                fromfile=f"qemu/{app.name}",
                tofile=f"orbe/{app.name}",
                lineterm="",
            )
        )
        return False, f"unexpected lines after periodic ticks:\n{diff}"

    q_pre_texts = [line for _, line in q_pre]
    o_pre_texts = [line for _, line in o_pre]
    if q_pre_texts != o_pre_texts:
        diff = "\n".join(
            difflib.unified_diff(
                q_pre_texts,
                o_pre_texts,
                fromfile=f"qemu/{app.name} (prefix)",
                tofile=f"orbe/{app.name} (prefix)",
                lineterm="",
            )
        )
        return False, f"prefix serial mismatch:\n{diff}"

    q_mean, q_err = check_periodic_rate("qemu", q_ticks, app.periodic)
    if q_err:
        return False, q_err
    o_mean, o_err = check_periodic_rate("orbe", o_ticks, app.periodic)
    if o_err:
        return False, o_err

    overlap = min(len(q_ticks), len(o_ticks))
    q_tick_texts = [line for _, line in q_ticks[:overlap]]
    o_tick_texts = [line for _, line in o_ticks[:overlap]]
    if q_tick_texts != o_tick_texts:
        diff = "\n".join(
            difflib.unified_diff(
                q_tick_texts,
                o_tick_texts,
                fromfile=f"qemu/{app.name} (ticks)",
                tofile=f"orbe/{app.name} (ticks)",
                lineterm="",
            )
        )
        return False, f"periodic sequence mismatch:\n{diff}"

    return True, (
        f"{len(q_pre_texts)} prefix line(s) + {overlap} periodic tick(s) match "
        f"(period: qemu {q_mean:.1f} ms, orbe {o_mean:.1f} ms, nominal {app.periodic.period_ms:.0f} ms)"
    )


def default_cache_dir() -> Path:
    env_cache = os.environ.get("ORBE_DIFF_SIM_CACHE")
    if env_cache:
        return Path(env_cache)
    xdg = os.environ.get("XDG_CACHE_HOME")
    base = Path(xdg) if xdg else Path.home() / ".cache"
    return base / "orbe-diff-sim"


def default_orbe_run() -> Path:
    env_bin = os.environ.get("ORBE_RUN")
    if env_bin:
        return Path(env_bin)
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")))
    for profile in ("release", "debug"):
        cand = target / profile / "orbe-run"
        if cand.is_file():
            return cand
    return target / "debug" / "orbe-run"


def discover_repo_s3_paths() -> set[str]:
    """Return every ESP32-S3 fixture, circuit, and DevKitC Block path in the repo."""
    found: set[str] = set()
    for d in sorted((ROOT / "packages/scene/tests/fixtures").glob("esp32s3*")):
        if d.is_dir():
            found.add(d.relative_to(ROOT).as_posix())
    for d in sorted((ROOT / "packages/engine/tests/circuits").glob("esp32s3*")):
        if d.is_dir():
            found.add(d.relative_to(ROOT).as_posix())
    for d in sorted((ROOT / "blocks").glob("devkitc-*")):
        if d.is_dir():
            found.add(d.relative_to(ROOT).as_posix())
    return found


def print_inventory() -> None:
    print(f"Covered ESP32-S3 apps ({len(APPS)}):")
    for app in APPS:
        suffix = f"  [{app.note}]" if app.note else ""
        print(f"  {app.name:30s} {app.rel_dir}{suffix}")
    print(f"\nExcluded ESP32-S3 apps ({len(EXCLUDED_APPS)}):")
    for rel, reason in sorted(EXCLUDED_APPS.items()):
        print(f"  {rel:52s} {reason}")


def parse_args(argv: list[str]) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument(
        "--app",
        action="append",
        dest="apps",
        default=[],
        help="Run only the named app (repeatable). Defaults to all covered apps.",
    )
    ap.add_argument("--list", action="store_true", help="List covered and excluded ESP32-S3 apps and exit.")
    ap.add_argument("--cache", type=Path, default=default_cache_dir(), help="Cache directory for QEMU and bootloaders.")
    ap.add_argument("--orbe-run", type=Path, default=default_orbe_run(), help="Path to orbe-run binary.")
    ap.add_argument(
        "--orbe-timeout",
        type=float,
        default=None,
        help="Override per-app wall-clock timeout in seconds for orbe-run.",
    )
    ap.add_argument("--blobs", type=Path, default=None, help="Path to orbe ROM blob cache directory.")
    return ap.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.list:
        print_inventory()
        return 0
    if args.orbe_timeout is not None and args.orbe_timeout <= 0:
        print("diff-esp32s3: --orbe-timeout must be positive", file=sys.stderr)
        return 2

    by_name = {a.name: a for a in APPS}
    if args.apps:
        unknown = [name for name in args.apps if name not in by_name]
        if unknown:
            print(f"diff-esp32s3: unknown app(s): {', '.join(unknown)}", file=sys.stderr)
            return 2
        selected = [by_name[name] for name in args.apps]
    else:
        selected = list(APPS)

    orbe_run = args.orbe_run
    if not orbe_run.is_file():
        print(
            f"diff-esp32s3: {orbe_run} not found "
            "(build with cargo build -p orbe-engine --features runner --bin orbe-run)",
            file=sys.stderr,
        )
        return 2

    if args.blobs is not None:
        rom_cache = args.blobs
    else:
        rom_cache = Path(
            subprocess.check_output([str(ROOT / "packages/engine/scripts/blobs.sh")], text=True)
            .strip()
            .splitlines()[-1]
        )

    qemu_bin = ensure_qemu(args.cache)
    lib_dir = ensure_userspace_libs(qemu_bin, args.cache)
    bootloaders = ensure_bootloaders(args.cache)

    qemu_ver_lines = subprocess.check_output(
        [str(qemu_bin), "--version"],
        env={**os.environ, "LD_LIBRARY_PATH": f"{lib_dir}:{os.environ.get('LD_LIBRARY_PATH', '')}".rstrip(":")},
        text=True,
    ).splitlines()
    qemu_ver = qemu_ver_lines[0] if qemu_ver_lines else str(qemu_bin)
    print(f"QEMU: {qemu_ver}", flush=True)
    print(f"orbe-run: {orbe_run}", flush=True)
    print(f"Running {len(selected)} ESP32-S3 app(s)...\n", flush=True)

    failures: list[tuple[str, str]] = []
    t0 = time.monotonic()

    with tempfile.TemporaryDirectory(prefix="orbe-flash-imgs-") as td:
        img_dir = Path(td)
        for app in selected:
            app_start = time.monotonic()
            part_path, app_path = resolve_app_images(app)
            flash_img = img_dir / f"{app.name}.bin"
            flash_mb = build_merged_flash(bootloaders[app.boot_mode], part_path, app_path, flash_img)

            qemu_raw, qemu_err, early_rc = run_qemu(qemu_bin, lib_dir, flash_img, app)
            if early_rc is not None:
                tail = _stderr_tail(qemu_err)
                detail = f"QEMU exited early with code {early_rc}"
                if tail:
                    detail += f"\nstderr:\n{tail}"
                failures.append((app.name, detail))
                print(
                    f"FAIL  {app.name:28s} ({flash_mb:2d}MB {app.boot_mode}) — QEMU exited early (rc={early_rc})",
                    flush=True,
                )
                if tail:
                    print(f"stderr:\n{tail}", flush=True)
                continue

            if not qemu_raw:
                tail = _stderr_tail(qemu_err)
                detail = f"QEMU timed out or produced no serial output within {app.qemu_timeout_s:g} s"
                if tail:
                    detail += f"\nstderr:\n{tail}"
                failures.append((app.name, detail))
                print(
                    f"FAIL  {app.name:28s} ({flash_mb:2d}MB {app.boot_mode}) — QEMU timed out",
                    flush=True,
                )
                print(detail, flush=True)
                continue

            try:
                orbe_raw, _report = run_orbe(
                    orbe_run,
                    rom_cache,
                    part_path,
                    app_path,
                    app,
                    orbe_timeout_s=args.orbe_timeout,
                )
            except OrbeTimeoutError as exc:
                failures.append((app.name, str(exc)))
                print(f"FAIL {app.name} orbe-run timed out after {exc.timeout_s:g} s", flush=True)
                if tail := _stderr_tail(exc.stderr):
                    print(f"stderr:\n{tail}", flush=True)
                continue
            except RuntimeError as exc:
                failures.append((app.name, str(exc)))
                print(
                    f"FAIL  {app.name:28s} ({flash_mb:2d}MB {app.boot_mode}) — orbe-run error",
                    flush=True,
                )
                print(exc, flush=True)
                continue

            ok, detail = compare_outputs(app, qemu_raw, orbe_raw)
            dt = time.monotonic() - app_start
            if ok:
                print(f"PASS  {app.name:28s} ({flash_mb:2d}MB {app.boot_mode}, {dt:4.2f}s) — {detail}", flush=True)
            else:
                failures.append((app.name, detail))
                print(f"FAIL  {app.name:28s} ({flash_mb:2d}MB {app.boot_mode}, {dt:4.2f}s)", flush=True)
                print(detail, flush=True)

    total_dt = time.monotonic() - t0
    passed = len(selected) - len(failures)
    print(f"\nSummary: {passed}/{len(selected)} passed, {len(failures)} failed in {total_dt:.1f}s", flush=True)
    if failures:
        print("Failed apps: " + ", ".join(name for name, _ in failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
