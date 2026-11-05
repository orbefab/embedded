"""The engine's seeded contact bounce, as crates/core/src/netlist/bounce.rs expands it."""

# The engine's least time between two edges of one switch.
MIN_GAP_PS = 2000
M64 = (1 << 64) - 1


def mix(z):
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
    return z ^ (z >> 31)


class SplitMix64:
    def __init__(self, state):
        self.state = state & M64

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & M64
        return mix(self.state)

    def below(self, n):
        """In [0, n): the high 64 bits of the 128-bit product."""
        return (self.next() * n) >> 64


def fnv1a64(data):
    h = 0xCBF29CE484222325
    for byte in data:
        h = ((h ^ byte) * 0x100000001B3) & M64
    return h


def bounce_stream(seed, name, index):
    return SplitMix64(mix(mix(seed ^ fnv1a64(name.encode())) ^ index))


def switch_edges(seed, part):
    """Every contact edge, nominal and bounce, ascending, as the engine's `bounce::expand`."""
    edges = []
    for index, event in enumerate(part["events"]):
        edges.append({"t_ps": event["t_ps"], "state": event["state"]})
        bounce = part["bounce"]
        if bounce is None:
            continue
        rng = bounce_stream(seed, part["name"], index)
        flips = 2 * (1 + rng.below(bounce["max_bounces"]["value"]))
        slack = bounce["window"]["value"] - flips * MIN_GAP_PS
        draws = sorted(rng.below(slack + 1) for _ in range(flips))
        state = event["state"]
        for j, draw in enumerate(draws, start=1):
            state = "open" if state == "closed" else "closed"
            edges.append({"t_ps": event["t_ps"] + draw + j * MIN_GAP_PS, "state": state})
    return edges


def bounce_vectors():
    """The Rust unit tests in crates/core/src/netlist/bounce.rs pin the same numbers."""
    rng = SplitMix64(0)
    first = [rng.next(), rng.next()]
    button = {
        "name": "sw1",
        "events": [
            {"t_ps": 2000000000, "state": "closed"},
            {"t_ps": 12000000000, "state": "open"},
        ],
        "bounce": {"window": {"value": 1000000000}, "max_bounces": {"value": 4}},
    }
    edges = [(edge["t_ps"], edge["state"][0]) for edge in switch_edges(7, button)]
    ok = (
        first == [0xE220A8397B1DCDAF, 0x6E789E6AA1B965F4]
        and fnv1a64(b"sw1") == 0x826ADE195D0A12BA
        and bounce_stream(7, "sw1", 0).state == 0xD65F52CA546DD32E
        and edges
        == [
            (2000000000, "c"),
            (2040015938, "o"),
            (2114234716, "c"),
            (2665597584, "o"),
            (2854176182, "c"),
            (12000000000, "o"),
            (12089763725, "c"),
            (12337840633, "o"),
            (12576192418, "c"),
            (12607405695, "o"),
            (12637715418, "c"),
            (12691361990, "o"),
            (12720009883, "c"),
            (12727823423, "o"),
        ]
    )
    return {"name": "bounce vectors", "ok": ok}
