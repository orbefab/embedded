# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fit the 1N4733A zener the examples use.

    uv run packages/engine/scripts/fit_zener.py

The forward points are stroke centers of Vishay 85816 Rev. 2.7, p3 Fig. 2,
Tj = 25 C, the 25 C trace, on a 200 dpi render of that page. x = 1013 px is
0 V and the scale is 410 px/V. The reverse targets are the reads of EIC
1N4727A to 1N4761A Fig. 1 at TA = 25 C: 4.46 V at 1 mA, 4.57 V at 5 mA,
4.66 V at 10 mA, and 5.08 V at 49 mA. VZ, IZT and ZZT are the Vishay p2
1N4733A row. The breakdown resistance is the one value that puts 1 mA on
4.46 V.
"""

import math
import sys

VT = 1.380649e-23 * 298.15 / 1.602176634e-19
X0 = 1013
PX_PER_V = 410
# (pixel, ampere). The fit uses these three. The other two are checks.
FORWARD_PX = [
    (1268, 1e-4),
    (1298, 1e-3),
    (1372, 0.1),
]
CHECK_PX = [
    (1211, 1e-6),
    (1332, 1e-2),
]
VZ = 5.1
IZT = 0.049
ZZT = 7.0
CURVE = [
    (0.001, 4.46),
    (0.005, 4.57),
    (0.010, 4.66),
    (0.049, 5.08),
]
PARENT_RB = 4.9
GMIN = 1e-12


def forward_points(pixels):
    return [((x - X0) / PX_PER_V, i) for x, i in pixels]


def residual(x, points):
    ln_is, n, rs = x
    is_ = math.exp(ln_is)
    out = []
    for volts, amps in points:
        out.append(n * VT * math.log1p(amps / is_) + amps * rs - volts)
    return out


def solve_forward(points):
    # Start near a hand solution of the same three points.
    x = [math.log(3e-13), 1.2, 0.4]
    for _ in range(40):
        base = residual(x, points)
        if max(abs(v) for v in base) < 1e-14:
            break
        step = [1e-6, 1e-6, 1e-6]
        columns = []
        for k in range(3):
            trial = x[:]
            trial[k] += step[k]
            columns.append([(residual(trial, points)[i] - base[i]) / step[k] for i in range(3)])
        # Solve J * dx = -base. J's column k is columns[k].
        a = [[columns[c][r] for c in range(3)] + [-base[r]] for r in range(3)]
        for col in range(3):
            pivot = max(range(col, 3), key=lambda r: abs(a[r][col]))
            a[col], a[pivot] = a[pivot], a[col]
            scale = a[col][col]
            if scale == 0.0:
                raise SystemExit("forward fit is singular")
            for row in range(col, 4):
                a[col][row] /= scale
            for row in range(3):
                if row == col:
                    continue
                factor = a[row][col]
                for k in range(col, 4):
                    a[row][k] -= factor * a[col][k]
        x = [x[k] + a[k][3] for k in range(3)]
    else:
        raise SystemExit("forward fit did not converge")
    return math.exp(x[0]), x[1], x[2]


def derived(rs, rb, vz, izt, zzt):
    nbvt = (zzt - rs - rb) * izt
    bvj = vz - izt * (rs + rb)
    isb = izt / math.expm1(bvj / nbvt)
    return nbvt, bvj, isb


def pin_voltage(rs, rb, vz, izt, zzt, amps):
    nbvt, _, isb = derived(rs, rb, vz, izt, zzt)
    return nbvt * math.log1p(amps / isb) + amps * (rs + rb)


def device_current(is_, nvt, vj):
    if vj >= -3.0 * nvt:
        return is_ * math.expm1(vj / nvt)
    arg = 3.0 * nvt / (vj * math.e)
    arg = arg * arg * arg
    return -is_ * (1.0 + arg)


def reverse_terminal(rs, rb, is_f, n_f, is_b, nbvt, volts):
    """Current entering the anode, cathode `volts` above the anode.

    Each junction current is the device law plus `GMIN` times its voltage.
    The returned terminal current includes that `GMIN`.
    """
    nvt = n_f * VT

    def parts(v0, v1):
        vjf = v0 - volts
        vjb = v1 - v0
        jf = device_current(is_f, nvt, vjf)
        jb = device_current(is_b, nbvt, vjb)
        gf = GMIN * vjf
        gb = GMIN * vjb
        r0 = -v0 / rs - (jf + gf) + (jb + gb)
        r1 = (volts - v1) / rb - (jb + gb)
        return r0, r1, -v0 / rs, jf, jb, gf, gb

    v0 = 0.0
    v1 = volts
    for _ in range(40):
        r0, r1, terminal, jf, jb, gf, gb = parts(v0, v1)
        if max(abs(r0), abs(r1)) < 1e-15:
            return terminal, jf, jb
        eps = 1e-6
        r0a, r1a, *_ = parts(v0 + eps, v1)
        r0b, r1b, *_ = parts(v0, v1 + eps)
        j00 = (r0a - r0) / eps
        j10 = (r1a - r1) / eps
        j01 = (r0b - r0) / eps
        j11 = (r1b - r1) / eps
        det = j00 * j11 - j01 * j10
        if det == 0.0 or not math.isfinite(det):
            raise SystemExit("reverse solve is singular")
        v0 += (-r0 * j11 + j01 * r1) / det
        v1 += (-j00 * r1 + r0 * j10) / det
    raise SystemExit("reverse solve did not converge")


def slope_at(rs, rb, vz, izt, zzt, amps):
    nbvt, _, isb = derived(rs, rb, vz, izt, zzt)
    return rs + rb + nbvt / (amps + isb)


def fit_rb(rs, target):
    lo = 0.0
    hi = ZZT - rs
    for _ in range(80):
        mid = 0.5 * (lo + hi)
        if pin_voltage(rs, mid, VZ, IZT, ZZT, 0.001) < target:
            lo = mid
        else:
            hi = mid
    return 0.5 * (lo + hi)


def percent(model, curve):
    return 100.0 * (model - curve) / curve


def main():
    points = forward_points(FORWARD_PX)
    is_, n, rs = solve_forward(points)
    print(f"VT {VT:.17g}")
    print("forward fit, exact through the three stroke centers")
    for volts, amps in points:
        print(f"  {amps * 1e3:g} mA at {volts:.6f} V")
    print(f"Is {is_:.17g} A")
    print(f"N {n:.17g}")
    print(f"Rs {rs:.17g} ohm")
    worst = max(abs(v) for v in residual([math.log(is_), n, rs], points))
    print(f"forward residual {worst:.3g} V")
    print("forward checks, not used in the fit")
    for volts, amps in forward_points(CHECK_PX):
        model = n * VT * math.log1p(amps / is_) + amps * rs
        print(f"  {amps * 1e3:g} mA read {volts:.4f} V model {model:.4f} V")
    vf = n * VT * math.log1p(0.2 / is_) + 0.2 * rs
    print(f"VF at 200 mA {vf:.6f} V, limit 1.2 V")

    rb = fit_rb(rs, 4.46)
    nbvt, bvj, isb = derived(rs, rb, VZ, IZT, ZZT)
    print("breakdown resistance fit to 4.46 V at 1 mA")
    print(f"Rb {rb:.17g} ohm")
    print(f"nbvt {nbvt:.17g} V")
    print(f"NBV {nbvt / VT:.17g}")
    print(f"bvj {bvj:.17g} V")
    print(f"Isb {isb:.17g} A")
    print("reverse, model against the knee reads")
    failed = False
    for amps, curve in CURVE:
        model = pin_voltage(rs, rb, VZ, IZT, ZZT, amps)
        err = percent(model, curve)
        print(f"  {amps * 1e3:g} mA curve {curve:.3f} V model {model:.4f} V err {err:+.3f} %")
        if abs(err) > 5.0:
            failed = True
    at_izt = pin_voltage(rs, rb, VZ, IZT, ZZT, IZT)
    zzt_model = slope_at(rs, rb, VZ, IZT, ZZT, IZT)
    print(f"at IZT model {at_izt:.6f} V slope {zzt_model:.9f} ohm")
    knee = slope_at(rs, rb, VZ, IZT, ZZT, 0.001)
    print(f"knee impedance at 1 mA {knee:.4f} ohm, maximum 550 ohm")
    terminal, jf, jb = reverse_terminal(rs, rb, is_, n, isb, nbvt, 1.0)
    leakage = abs(terminal)
    print(f"leakage at 1 V, current entering the anode {terminal:.16e} A, GMIN included, maximum 10 uA")
    print(f"  forward device {jf:.16e} A, breakdown device {jb:.16e} A")
    print("estimate Rb = 4.9 ohm, beside the fit")
    for amps, curve in CURVE:
        model = pin_voltage(rs, PARENT_RB, VZ, IZT, ZZT, amps)
        err = percent(model, curve)
        print(f"  {amps * 1e3:g} mA model {model:.4f} V err {err:+.3f} %")
    parent_knee = slope_at(rs, PARENT_RB, VZ, IZT, ZZT, 0.001)
    print(f"estimate knee impedance at 1 mA {parent_knee:.4f} ohm")

    if vf >= 1.2 or knee > 550.0 or leakage > 10e-6 or abs(at_izt - VZ) > 1e-9:
        failed = True
    if abs(zzt_model - ZZT) > 1e-6:
        failed = True
    print("gate", "fail" if failed else "pass")
    if failed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
