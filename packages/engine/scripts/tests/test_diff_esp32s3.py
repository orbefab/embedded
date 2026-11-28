"""Unit tests for scripts/diff_esp32s3.py (normalization, panic detection, periodic rate, inventory)."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "scripts"))

import diff_esp32s3 as diff  # noqa: E402


def _make_app(*, periodic: diff.PeriodicSpec | None = None) -> diff.AppSpec:
    return diff.AppSpec(
        name="test-app",
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-blink",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=500,
        qemu_timeout_s=2.0,
        periodic=periodic,
    )


def test_normalize_strips_banner_timestamps_chip_rev_and_flash_chip() -> None:
    qemu_raw = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "Build:Mar 27 2021"),
        (0.01, "rst:0x1 (POWERON),boot:0x4 (SPI_FLASH_BOOT)"),
        (0.01, "entry 0x403c88b8"),
        (0.05, "W (117) rtcinit: o_code calibration fail"),
        (0.05, ""),
        (0.06, "I (62) cpu_start: Unicore app"),
        (0.07, "I (73) efuse_init: Chip rev:         v0.0"),
        (0.08, "I (75) spi_flash: detected chip: issi"),
        (0.09, "This is esp32s3 chip with 2 CPU core(s), WiFi/BLE, silicon revision v0.0, 16MB external flash"),
        (0.10, "Hello world!"),
    ]
    orbe_raw = [
        (0.06, "I (59) cpu_start: Unicore app"),
        (0.07, "I (68) efuse_init: Chip rev:         v0.2"),
        (0.08, "I (69) spi_flash: detected chip: boya"),
        (0.09, "This is esp32s3 chip with 2 CPU core(s), WiFi/BLE, silicon revision v0.2, 16MB external flash"),
        (0.10, "Hello world!"),
    ]
    ok, _ = diff.compare_outputs(_make_app(), qemu_raw, orbe_raw)
    assert ok


def test_real_mismatch_fails_with_unified_diff() -> None:
    qemu_raw = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "entry 0x403c8898"),
        (0.10, "nvs ok=1 count=99 tag=after"),
    ]
    orbe_raw = [
        (0.10, "nvs ok=1 count=42 tag=after"),
    ]
    ok, msg = diff.compare_outputs(_make_app(), qemu_raw, orbe_raw)
    assert not ok
    assert "--- qemu/test-app" in msg
    assert "+++ orbe/test-app" in msg
    assert "-nvs ok=1 count=99 tag=after" in msg
    assert "+nvs ok=1 count=42 tag=after" in msg


def test_detects_panic_watchdog_and_second_boot_banner() -> None:
    for bad_line in (
        "Guru Meditation Error: Core  0 panic'ed (LoadProhibited).",
        "abort() was called at PC 0x42014d3b on core 1",
        "E (5000) task_wdt: Task watchdog got triggered.",
        "rst:0x7 (TG0WDT_SYS_RST),boot:0x4 (SPI_FLASH_BOOT)",
        "Rebooting...",
    ):
        qemu_raw = [(0.01, "ESP-ROM:esp32s3-20210327"), (0.01, "entry 0x403c8898"), (0.10, bad_line)]
        ok, msg = diff.compare_outputs(_make_app(), qemu_raw, [(0.10, "ready")])
        assert not ok, bad_line
        assert "fatal" in msg

    reboot_raw = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "entry 0x403c8898"),
        (0.10, "ready"),
        (0.50, "ESP-ROM:esp32s3-20210327"),
        (0.50, "entry 0x403c8898"),
    ]
    ok, msg = diff.compare_outputs(_make_app(), reboot_raw, [(0.10, "ready")])
    assert not ok
    assert "second ROM banner" in msg


def test_periodic_rate_check_accepts_within_25_percent_and_rejects_outside() -> None:
    app = _make_app(periodic=diff.PeriodicSpec(pattern=re.compile(r"^tick$"), period_ms=100.0, min_ticks=4))
    qemu_ok = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "entry 0x403c8898"),
        (0.05, "ready"),
        (0.10, "tick"),
        (0.21, "tick"),
        (0.32, "tick"),
        (0.43, "tick"),
    ]
    orbe_ok = [
        (0.01, "ready"),
        (0.02, "tick"),
        (0.12, "tick"),
        (0.22, "tick"),
        (0.32, "tick"),
        (0.42, "tick"),
    ]
    ok, _ = diff.compare_outputs(app, qemu_ok, orbe_ok)
    assert ok

    # 140 ms period is > +25% above 100 ms -> must fail.
    qemu_slow = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "entry 0x403c8898"),
        (0.05, "ready"),
        (0.10, "tick"),
        (0.24, "tick"),
        (0.38, "tick"),
        (0.52, "tick"),
    ]
    ok, msg = diff.compare_outputs(app, qemu_slow, orbe_ok)
    assert not ok
    assert "outside ±25%" in msg


def test_timeout_empty_output_fails() -> None:
    qemu_raw = [(0.01, "ESP-ROM:esp32s3-20210327"), (0.01, "entry 0x403c8898")]
    ok, msg = diff.compare_outputs(_make_app(), qemu_raw, [(0.05, "ready")])
    assert not ok
    assert "timed out" in msg


def test_every_repo_esp32s3_app_is_covered_or_excluded() -> None:
    covered = {a.rel_dir for a in diff.APPS}
    excluded = set(diff.EXCLUDED_APPS)
    assert covered.isdisjoint(excluded), f"overlap: {covered & excluded}"
    discovered = diff.discover_repo_s3_paths()
    accounted = covered | excluded
    missing = discovered - accounted
    stale = accounted - discovered
    assert not missing, f"unaccounted ESP32-S3 apps: {sorted(missing)}"
    assert not stale, f"stale ESP32-S3 entries: {sorted(stale)}"
