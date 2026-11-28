"""Unit tests for scripts/diff_esp32s3.py (normalization, panic detection, periodic rate, inventory)."""

from __future__ import annotations

import os
import re
import stat
import sys
import time
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "scripts"))

import diff_esp32s3 as diff  # noqa: E402


def _make_app(
    *,
    name: str = "test-app",
    periodic: diff.PeriodicSpec | None = None,
    qemu_timeout_s: float = 2.0,
    orbe_timeout_s: float = 2.0,
) -> diff.AppSpec:
    return diff.AppSpec(
        name=name,
        kind="fixture",
        rel_dir="packages/scene/tests/fixtures/esp32s3-stock-blink",
        boot_mode="dio",
        orbe_usart="UART0",
        orbe_stop_ms=500,
        qemu_timeout_s=qemu_timeout_s,
        orbe_timeout_s=orbe_timeout_s,
        periodic=periodic,
    )


def _write_executable(path: Path, body: str) -> Path:
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return path


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


def test_in_app_silicon_revision_mismatch_is_not_normalized() -> None:
    qemu_raw = [
        (0.01, "ESP-ROM:esp32s3-20210327"),
        (0.01, "entry 0x403c88b8"),
        (0.10, "board silicon revision v1.2 id=abc"),
    ]
    orbe_raw = [
        (0.10, "board silicon revision v9.9 id=abc"),
    ]
    ok, msg = diff.compare_outputs(_make_app(), qemu_raw, orbe_raw)
    assert not ok
    assert "-board silicon revision v1.2 id=abc" in msg
    assert "+board silicon revision v9.9 id=abc" in msg

    # Other fields on the IDF hello_world chip-info line are still compared.
    ok2, msg2 = diff.compare_outputs(
        _make_app(),
        [
            (0.01, "entry 0x403c88b8"),
            (0.10, "This is esp32s3 chip with 2 CPU core(s), WiFi/BLE, silicon revision v0.0, 16MB external flash"),
        ],
        [
            (0.10, "This is esp32s3 chip with 2 CPU core(s), WiFi/BLE, silicon revision v0.2, 4MB external flash"),
        ],
    )
    assert not ok2
    assert "16MB external flash" in msg2
    assert "4MB external flash" in msg2


def test_hanging_orbe_run_times_out_and_reports_fail(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    hang_orbe = _write_executable(tmp_path / "hang-orbe.sh", "#!/bin/sh\nsleep 60\n")
    part_path, app_path = diff.resolve_app_images(diff.APPS[1])
    app = _make_app(name="hang-app", orbe_timeout_s=0.3)

    t0 = time.monotonic()
    with pytest.raises(diff.OrbeTimeoutError) as exc_info:
        diff.run_orbe(hang_orbe, tmp_path, part_path, app_path, app)
    assert time.monotonic() - t0 < 5.0
    assert "orbe-run timed out after 0.3 s" in str(exc_info.value)

    fake_boot = tmp_path / "boot.bin"
    fake_boot.write_bytes(b"\xe9" + b"\x00" * 63)
    fake_qemu = _write_executable(tmp_path / "fake-qemu.sh", '#!/bin/sh\necho "fake qemu 1.0"\n')

    monkeypatch.setattr(diff, "ensure_qemu", lambda _cache: fake_qemu)
    monkeypatch.setattr(diff, "ensure_userspace_libs", lambda _q, _c: tmp_path)
    monkeypatch.setattr(diff, "ensure_bootloaders", lambda _c: {"dio": fake_boot, "qio": fake_boot})
    monkeypatch.setattr(
        diff,
        "run_qemu",
        lambda _q, _l, _f, _a: ([(0.01, "entry 0x403c88b8"), (0.05, "ready")], "", None),
    )

    rc = diff.main(
        [
            "--app",
            "esp32s3-stock-blink",
            "--orbe-run",
            str(hang_orbe),
            "--orbe-timeout",
            "0.3",
            "--blobs",
            str(tmp_path),
        ]
    )
    captured = capsys.readouterr()
    assert rc == 1
    assert "FAIL esp32s3-stock-blink orbe-run timed out after 0.3 s" in captured.out
    assert "Summary: 0/1 passed, 1 failed" in captured.out


def test_qemu_exit_before_connect_reports_fail_and_continues(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    marker = tmp_path / "qemu-count"
    fake_qemu = _write_executable(
        tmp_path / "fake-qemu.sh",
        "#!/bin/sh\n"
        'if [ "$1" = "--version" ]; then\n'
        '  echo "fake-qemu 0.0"\n'
        "  exit 0\n"
        "fi\n"
        "echo 'qemu: init failed line 1' >&2\n"
        "echo 'qemu: init failed line 2' >&2\n"
        "exit 7\n",
    )
    flash_img = tmp_path / "flash.bin"
    flash_img.write_bytes(b"\xff" * 4096)

    t0 = time.monotonic()
    lines, stderr, early_rc = diff.run_qemu(fake_qemu, tmp_path, flash_img, _make_app(qemu_timeout_s=5.0))
    assert time.monotonic() - t0 < 2.0
    assert lines == []
    assert early_rc == 7
    assert "qemu: init failed line 2" in stderr

    # End-to-end through main() with two apps: first exits 7 before connecting, second runs and passes.
    # Use a stateful fake QEMU that fails on the first app and connects + emits serial on the second.
    stateful_qemu = _write_executable(
        tmp_path / "stateful-qemu.py",
        f"#!{sys.executable}\n"
        "import socket, sys\n"
        'if "--version" in sys.argv:\n'
        '    print("fake-qemu 0.0")\n'
        "    raise SystemExit(0)\n"
        f"marker = {str(marker)!r}\n"
        "import pathlib\n"
        "p = pathlib.Path(marker)\n"
        "if not p.exists():\n"
        '    p.write_text("1")\n'
        '    sys.stderr.write("qemu: fatal early exit\\n")\n'
        "    raise SystemExit(7)\n"
        'idx = sys.argv.index("-serial") + 1\n'
        'sock_path = sys.argv[idx].removeprefix("unix:")\n'
        "s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n"
        "s.connect(sock_path)\n"
        's.sendall(b"ESP-ROM:esp32s3-20210327\\nentry 0x403c88b8\\nready\\n")\n'
        "import time\n"
        "time.sleep(2)\n",
    )
    fake_boot = tmp_path / "boot.bin"
    fake_boot.write_bytes(b"\xe9" + b"\x00" * 63)
    fake_orbe = _write_executable(tmp_path / "fake-orbe.sh", "#!/bin/sh\nexit 0\n")

    monkeypatch.setattr(diff, "ensure_qemu", lambda _cache: stateful_qemu)
    monkeypatch.setattr(diff, "ensure_userspace_libs", lambda _q, _c: tmp_path)
    monkeypatch.setattr(diff, "ensure_bootloaders", lambda _c: {"dio": fake_boot, "qio": fake_boot})
    monkeypatch.setattr(
        diff,
        "run_orbe",
        lambda *_a, **_kw: ([(0.05, "ready")], {"errors": 0}),
    )

    rc = diff.main(
        [
            "--app",
            "esp32s3-stock-blink",
            "--app",
            "esp32s3-stock-wire",
            "--orbe-run",
            str(fake_orbe),
            "--blobs",
            str(tmp_path),
        ]
    )
    captured = capsys.readouterr()
    assert rc == 1
    assert "FAIL" in captured.out
    assert "esp32s3-stock-blink" in captured.out
    assert "rc=7" in captured.out
    assert "qemu: fatal early exit" in captured.out
    assert "PASS" in captured.out
    assert "esp32s3-stock-wire" in captured.out
    assert "Summary: 1/2 passed, 1 failed" in captured.out
    assert os.path.exists(marker)
