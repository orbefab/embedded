"""What `ws2812_decode` accepts and what it refuses."""

from ws2812_decode import RES, frames, runs


def test_power_on_low_is_not_a_frame() -> None:
    """A power-on low longer than RES, then one LED of 0x80 0x00 0x00 and a reset."""
    start = 2 * RES
    samples = [(0, False)]
    at = start
    for k in range(24):
        high = 625_000 if k == 0 else 312_500
        samples += [(at, True), (at + high, False)]
        at += 1_250_000
    fall = samples[-1][0]
    stop = fall + 2 * RES
    want = [(fall + RES + 1, [1] + [0] * 23)]
    got = frames(runs(samples, stop))
    assert got == want
