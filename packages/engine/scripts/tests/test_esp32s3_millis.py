"""What `esp32s3_millis` accepts and what it refuses."""

from esp32s3_millis import PERIOD, SLACK, check_edges

STOP = 2_000_000_000_000
FIRST = 505_274_875_000


def vcd_of(changes: list[tuple[int, str]]) -> str:
    head = "$timescale 1 ps $end\n$scope module u1 $end\n$var wire 1 ! GPIO4 $end\n"
    head += "$upscope $end\n$enddefinitions $end\n#0\n$dumpvars\n0!\n$end\n"
    return head + "".join(f"#{at}\n{level}!\n" for at, level in changes)


def must_fail(what: str, run) -> None:
    try:
        run()
    except ValueError:
        return
    raise AssertionError(f"{what} passed")


def test_edges() -> None:
    good = [(FIRST + k * PERIOD + 600_000 * (k % 2), "0" if k % 2 else "1") for k in range(3)]
    assert check_edges(vcd_of(good), STOP) == 3

    must_fail("a missing edge", lambda: check_edges(vcd_of(good[:-1]), STOP))
    must_fail("an extra edge", lambda: check_edges(vcd_of(good + [(FIRST + 3 * PERIOD, "0")]), STOP))
    falling = [(at, "0" if level == "1" else "1") for at, level in good]
    must_fail("falling first", lambda: check_edges(vcd_of(falling), STOP))
    late = good[:2] + [(good[1][0] + PERIOD + SLACK + 1, good[2][1])]
    must_fail("an edge past the slack", lambda: check_edges(vcd_of(late), STOP))
    ms = [(at + 1_000_000_000 * k, level) for k, (at, level) in enumerate(good)]
    must_fail("a millisecond drift", lambda: check_edges(vcd_of(ms), STOP))
    early = [(at - 10_000_000_000, level) for at, level in good]
    must_fail("a first edge before 500 ms", lambda: check_edges(vcd_of(early), STOP))
