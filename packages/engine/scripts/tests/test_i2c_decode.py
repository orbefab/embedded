"""What `i2c_decode` accepts and what it refuses."""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

import i2c_decode
from i2c_decode import S3_EDGES, TABLE, s3_lengths, s3_problems


def expect(name, ok):
    assert ok, name


def test_lengths_and_views():
    """The S3 lengths, and that an ATmega view adds no line for a matching controller."""

    def seen_of(**rows):
        base = {name: [] for name in (*TABLE,)}
        base.update(rows)
        return base

    edge = 1_250_000
    exact = seen_of(**{name: [edge] for name in S3_EDGES}, **{"tHD;DAT": [300_000], "tSU;DAT": [950_000]})
    expect("400 kHz lengths pass", s3_problems(exact, 400_000) == [])
    expect("a tLOW 1 ps off fails", "tLOW" in s3_problems({**exact, "tLOW": [edge + 1]}, 400_000))
    expect("an empty interval passes", s3_problems(seen_of(), 400_000) == [])
    expect("tBUF is not checked", s3_problems({**exact, "tBUF": [33_000_000_000]}, 400_000) == [])
    slow = seen_of(
        **{name: [5_000_000] for name in S3_EDGES},
        **{"tHD;DAT": [1_250_000, 300_000], "tSU;DAT": [3_750_000, 4_700_000]},
    )
    expect("100 kHz takes the master hold and the 300 ns hold", s3_problems(slow, 100_000) == [])
    expect("a 400 ns hold fails at 100 kHz", "tHD;DAT" in s3_problems({**slow, "tHD;DAT": [400_000]}, 100_000))
    expect(
        "a setup that is not the low minus its hold fails",
        "tSU;DAT" in s3_problems({**slow, "tSU;DAT": [950_000]}, 100_000),
    )
    expect("another rate is rejected", s3_lengths(200_000) is None)

    def vcd(scopes):
        body = ["$timescale 1 ps $end"]
        n = 0
        codes = []
        for scope, names in scopes:
            body.append(f"$scope module {scope} $end")
            for name in names:
                code = chr(ord("!") + n)
                n += 1
                codes.append(code)
                body.append(f"$var wire 1 {code} {name} $end")
            body.append("$upscope $end")
        body += ["$enddefinitions $end", "$dumpvars"]
        body += [f"1{code}" for code in codes]
        body.append("$end")
        return "\n".join(body) + "\n"

    with tempfile.TemporaryDirectory() as raw:
        folder = Path(raw)
        (folder / "traces").mkdir()
        (folder / "traces" / "i2c.csv").write_text("t_ps,part,phase,address,read,byte,ack\n")
        (folder / "traces" / "logic.vcd").write_text(
            vcd(
                [
                    ("u1", ["PC5", "PC4"]),
                    ("u2", ["SCL", "SDA"]),
                    ("u3", ["GPIO10", "GPIO9"]),
                ]
            )
        )
        netlist = {
            "parts": [
                {"name": "u1", "kind": "atmega328p", "clock_hz": {"value": 16_000_000}, "pins": {}},
                {"name": "u2", "kind": "cst816", "pins": {"SCL": "scl", "SDA": "sda"}},
                {"name": "u3", "kind": "esp32s3", "pins": {"GPIO10": "scl", "GPIO9": "sda"}},
            ]
        }
        (folder / "netlist.json").write_text(json.dumps(netlist))
        script = Path(i2c_decode.__file__).resolve()
        out = subprocess.run([sys.executable, script, str(folder)], capture_output=True, text=True)
        expect(
            "a matching controller adds no line",
            out.returncode == 0
            and out.stdout == ("u1: 0 decoded phases, 0 rows in i2c.csv\nu1: 0 of 0 rows match, 0 to the picosecond\n"),
        )
        netlist["parts"] = netlist["parts"][1:]
        (folder / "netlist.json").write_text(json.dumps(netlist))
        out = subprocess.run([sys.executable, script, str(folder)], capture_output=True, text=True)
        expect(
            "an S3 run compares its views and does not open i2c.csv",
            out.returncode == 0 and out.stdout == ("u2 and u3 decode the same phases: True\n"),
        )
        (folder / "traces" / "i2c.csv").unlink()
        out = subprocess.run(
            [sys.executable, script, str(folder), "--timing-s3", "400000"], capture_output=True, text=True
        )
        expect("timing-s3 on an idle bus passes", out.returncode == 0 and "off=1" not in out.stdout)
        out = subprocess.run(
            [sys.executable, script, str(folder), "--timing-s3", "200000"], capture_output=True, text=True
        )
        expect("another timing-s3 rate exits 1", out.returncode == 1)
