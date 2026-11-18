"""What `esp32s3_timers` accepts and what it refuses."""

from esp32s3_timers import (
    BOARD_TOLERANCE,
    FIRST_STEP_SLACK,
    check_console,
    check_diagnostics,
    check_manifest,
    compare,
    period,
    rows_of,
)


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def console(shot_overhead: int = 500, reload_overhead: int = 800, rng: str = "nz") -> str:
    """A model-shaped console. One-shots are equal. Reload steps are the closed-form period."""
    lines = ["boot log\n"]
    for divider in (2, 80):
        span = period(divider)
        for index in range(8):
            lines.append(f"div {divider} shot {index} {span + shot_overhead}\n")
        for index in range(8):
            lines.append(f"div {divider} reload {index} {span + reload_overhead + index * span}\n")
    for index in range(8):
        lines.append(f"rng {index} {rng}\n")
    lines.append("TIMERS DONE\n")
    return "".join(lines)


def test_periods_and_board() -> None:
    good = console()
    expect("a closed-form console passes", not raises(lambda: check_console(good)))
    span = period(2)
    third = span + 800 + 3 * span
    late = good.replace(f"div 2 reload 3 {third}", f"div 2 reload 3 {third + 1}", 1)
    expect("a reload a cycle late fails", raises(lambda: check_console(late)))

    def shift_first(slack: int) -> str:
        out = []
        for line in good.splitlines(keepends=True):
            parts = line.split()
            if (
                len(parts) == 5
                and parts[0] == "div"
                and parts[1] == "2"
                and parts[2] == "reload"
                and int(parts[3]) >= 1
            ):
                parts[4] = str(int(parts[4]) - slack)
                line = " ".join(parts) + "\n"
            out.append(line)
        return "".join(out)

    expect("a first step 32 short passes", not raises(lambda: check_console(shift_first(FIRST_STEP_SLACK))))
    expect("a first step 33 short fails", raises(lambda: check_console(shift_first(FIRST_STEP_SLACK + 1))))
    short = console(shot_overhead=0)
    expect("a one-shot no longer than its period fails", raises(lambda: check_console(short)))
    wide = console(shot_overhead=5000)
    expect("a one-shot 5000 cycles over its period fails", raises(lambda: check_console(wide)))
    zero = console(rng="z")
    expect("a zero RNG word fails", raises(lambda: rows_of(zero)))
    expect("a missing TIMERS DONE fails", raises(lambda: rows_of(good.replace("TIMERS DONE\n", ""))))

    board = console(shot_overhead=500 + BOARD_TOLERANCE, reload_overhead=800 + BOARD_TOLERANCE)
    expect("a board within 4000 cycles passes", not raises(lambda: compare(good, board)))
    uneven = good.replace(f"div 2 shot 1 {span + 500}", f"div 2 shot 1 {span + 503}", 1)
    expect("a board whose shots differ still passes", not raises(lambda: compare(good, uneven)))
    over = console(shot_overhead=500 + BOARD_TOLERANCE + 1)
    expect("a board 4001 cycles off fails", raises(lambda: compare(good, over)))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
