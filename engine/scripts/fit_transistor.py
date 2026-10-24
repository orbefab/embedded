# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fit the `npn` and `pnp` examples' parameters from their datasheets' "ON" voltage figures.

    uv run scripts/fit_transistor.py

NF and NR are 1. BF is the geometric mean of the guaranteed hFE at 10 mA and VCE = 1 V, and
VAF is 100 V. Each typical reading then fixes one parameter in the region where it dominates,
and the four fits repeat to a fixed point.

1. BR from VCE(sat) at 1 mA, IC/IB = 10, where the ohmic drops are under 3 mV.
2. IS and RE + RB/BF from VBE at VCE = 1 V, at 1 mA and at 100 mA.
3. RB from VBE(sat) at 100 mA, IC/IB = 10.
4. RC from VCE(sat) at 100 mA, IC/IB = 10.

It prints each fit, rounded to the four figures the netlists carry, and the model's voltages at
points it was not fitted to, beside the datasheet's readings and table limits.
"""

import math

VT = 1.380649e-23 * 298.15 / 1.602176634e-19
VAF = 100.0
BF = math.sqrt(100 * 300)

# Volts read off each figure at TJ = 25 C. `vbe` is at VCE = 1.0 V, the rest at IC/IB = 10.
PARTS = {
    "npn": {
        "doc": "2N3903/D Rev. 8 Figure 17, the 2N3904 row's hFE 100 to 300",
        "vbe": {1e-3: 0.657, 0.1: 0.865},
        "vbesat_100m": 0.93,
        "vcesat_100m": 0.346,
        "vcesat_1m": 0.100,
        "vbe_10m": 0.720,
        "table": "VBE(sat) 0.65 to 0.85 V, VCE(sat) at most 0.2 V; at 50 mA at most 0.95 V and 0.3 V",
    },
    "pnp": {
        "doc": "2N3906/D Rev. 4 Figure 15, hFE 100 to 300",
        "vbe": {1e-3: 0.683, 0.1: 0.853},
        "vbesat_100m": 0.90,
        "vcesat_100m": 0.232,
        "vcesat_1m": 0.075,
        "vbe_10m": 0.728,
        "table": "VBE(sat) 0.65 to 0.85 V, VCE(sat) at most 0.25 V; at 50 mA at most 0.95 V and 0.4 V",
    },
}


def internal(ic, ib, br, is_):
    """The junction voltages that carry terminal currents `ic` and `ib`, iterating on the
    Early factor."""
    q = 1.0
    for _ in range(50):
        # ic = (ibe - ibc) * q - ibc / BR, ib = ibe / BF + ibc / BR
        a = [[q, -q - 1 / br], [1 / BF, 1 / br]]
        det = a[0][0] * a[1][1] - a[0][1] * a[1][0]
        ibe = (ic * a[1][1] - a[0][1] * ib) / det
        ibc = (a[0][0] * ib - a[1][0] * ic) / det
        vbc = VT * math.log(ibc / is_ + 1) if ibc > -is_ else -1.0
        q = 1 - vbc / VAF
    return VT * math.log(ibe / is_ + 1), vbc


def on(ic, vbe, p):
    """VBE at the pins at VCE = 1 V, from a starting junction voltage `vbe`."""
    for _ in range(50):
        vce = 1.0 - ic * p["RC"] - ic * (1 + 1 / BF) * p["RE"]
        q = 1 - (vbe - vce) / VAF
        vbe = VT * math.log(ic / (p["IS"] * q) + 1)
    return vbe + ic / BF * p["RB"] + ic * (1 + 1 / BF) * p["RE"]


def saturated(ic, ib, p):
    """VBE(sat) and VCE(sat) at the pins."""
    vbe, vbc = internal(ic, ib, p["BR"], p["IS"])
    emitter = (ic + ib) * p["RE"]
    return vbe + ib * p["RB"] + emitter, vbe - vbc + ic * p["RC"] + emitter


def fit(r):
    p = {"BR": 0.3, "IS": 1e-14, "RE": 0.5, "RB": 5.0, "RC": 1.0}
    for _ in range(200):
        lo, hi = 1e-3, 100.0
        for _ in range(200):
            p["BR"] = math.sqrt(lo * hi)
            vbe, vbc = internal(1e-3, 1e-4, p["BR"], p["IS"])
            v = vbe - vbc + 1e-3 * p["RC"] + 1.1e-3 * p["RE"]
            lo, hi = (p["BR"], hi) if v > r["vcesat_1m"] else (lo, p["BR"])
        for _ in range(100):
            low, high = on(1e-3, r["vbe"][1e-3], p), on(0.1, r["vbe"][0.1], p)
            p["IS"] *= math.exp((low - r["vbe"][1e-3]) / VT)
            p["RE"] += (r["vbe"][0.1] - high) / 0.1
        vbe, vbc = internal(0.1, 0.01, p["BR"], p["IS"])
        p["RB"] = (r["vbesat_100m"] - vbe - 0.11 * p["RE"]) / 0.01
        p["RC"] = (r["vcesat_100m"] - (vbe - vbc) - 0.11 * p["RE"]) / 0.1
    return p


def main():
    for kind, r in PARTS.items():
        p = fit(r)
        print(f"{kind}: {r['doc']}, BF {BF:.4g}, VAF {VAF:g} V, NF = NR = 1")
        for key, unit in (("IS", "A"), ("BR", ""), ("RB", "ohm"), ("RC", "ohm"), ("RE", "ohm")):
            print(f"  {key} {p[key]:.4g} {unit}".rstrip())
        rounded = {key: float(f"{value:.4g}") for key, value in p.items()}
        print(f"  VBE(on) at 10 mA {on(1e-2, r['vbe'][1e-3], rounded):.3f} V, read {r['vbe_10m']:.3f} V")
        for ic in (1e-2, 5e-2):
            vbe, vce = saturated(ic, ic / 10, rounded)
            print(f"  VBE(sat) and VCE(sat) at {ic * 1e3:g} mA, {ic * 1e2:g} mA: {vbe:.3f} V and {vce:.3f} V")
        print(f"  table {r['table']}")


if __name__ == "__main__":
    main()
