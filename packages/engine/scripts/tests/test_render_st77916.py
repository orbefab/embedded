"""What `render_st77916` accepts and what it refuses."""

import hashlib
import json
from pathlib import Path

from render_st77916 import (
    CS_PIN,
    HEADER,
    LINK_HEADER,
    PATTERN,
    compare,
    compare_link,
    link_rows,
    pattern_events,
    render,
)


def entry_for(state, sha):
    return {"sha256": sha, **state}


def replay(opcode, command, data, lanes, cs):
    phases = []
    if opcode is not None:
        phases = [
            {"bits": 8, "lanes": 1, "value": opcode},
            {"bits": 24, "lanes": 1, "value": command << 8},
        ]
    return {"phases": phases, "lanes": lanes, "bytes": list(data), "cs": cs}


def last_rectangle(events):
    """The first pixel transaction of the last rectangle."""
    pixel_writes = [event for event in events if event["phases"] and event["phases"][0]["value"] == 0x32]
    return pixel_writes[-1]


def test_replay_compare() -> None:
    events = json.loads(
        (Path(__file__).resolve().parents[4] / "packages/engine/tests/circuits/st77916-replay/events.json").read_text()
    )
    rows, image, state = render(events)
    assert len(rows) >= 2, rows
    sha = hashlib.sha256(image).hexdigest()
    run_csv = (
        "t_ps,"
        + ",".join(HEADER)
        + "\n"
        + "".join(f"{k},lcd1,{','.join(map(str, row))}\n" for k, row in enumerate(rows))
    )
    entry = entry_for(state, sha)
    assert compare("lcd1", rows, image, state, run_csv, image, entry) == [], "the right run passes"
    blank = bytes(len(image))
    assert compare("lcd1", rows, image, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), (
        "a blank frame fails"
    )
    missing, _, _ = render(events[:-1])
    assert len(missing) == len(rows) - 1
    _, short, _ = render([event for event in events if event is not last_rectangle(events)])
    assert short != image
    assert compare("lcd1", rows, image, state, run_csv, short, entry_for(state, hashlib.sha256(short).hexdigest())), (
        "a frame missing a rectangle fails"
    )
    dropped = "\n".join(run_csv.splitlines()[:-1]) + "\n"
    assert compare("lcd1", rows, image, state, dropped, image, entry), "a missing row fails"
    assert compare("lcd1", [], image, state, run_csv, image, entry), "no expected row fails"
    assert compare("lcd1", rows, blank, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), (
        "a blank expected image fails"
    )
    bad_registers = entry_for(state, sha)
    bad_registers["registers"] = state["registers"] + 1
    assert compare("lcd1", rows, image, state, run_csv, image, bad_registers), "a wrong registers count fails"
    bad_flags = entry_for(state, sha)
    bad_flags["flags"] = {**state["flags"], "display_on": not state["flags"]["display_on"]}
    assert compare("lcd1", rows, image, state, run_csv, image, bad_flags), "wrong flags fail"
    bad_madctl = entry_for(state, sha)
    bad_madctl["madctl"] = state["madctl"] ^ 0x40
    assert compare("lcd1", rows, image, state, run_csv, image, bad_madctl), "a wrong madctl fails"
    bad_colmod = entry_for(state, sha)
    bad_colmod["colmod"] = state["colmod"] ^ 0x01
    assert compare("lcd1", rows, image, state, run_csv, image, bad_colmod), "a wrong colmod fails"
    colmod = replay(0x02, 0x3A, [0x55], 1, "released")
    alone = replay(0x32, 0x2C, [], 1, "held")
    pixels = replay(None, 0, [0xAB, 0xCD], 4, "released")
    drawn, frame, _ = render([colmod, alone, pixels])
    assert len(drawn) == 1 and drawn[0][4] == 2, "a RAMWR with no bytes draws the following data"
    assert frame[:2] == bytes([0xAB, 0xCD])
    _, _, on = render([replay(0x02, 0x29, [], 4, "released")])
    assert on["flags"]["display_on"], "a DISPON with no bytes runs"
    _, _, off = render([replay(0x32, 0x29, [], 1, "released")])
    assert not off["flags"]["display_on"], "a data-less command still checks its opcode"
    held = replay(0x32, 0x2C, [0x11, 0x22], 4, "held")
    bad = replay(None, 0, [0x33, 0x44], 1, "held")
    good = replay(None, 0, [0x55, 0x66], 4, "released")
    rows_b, frame_b, _ = render([colmod, held, bad, good])
    assert len(rows_b) == 1 and rows_b[0][4] == 4, "a continuation on the wrong lanes drops its bytes"
    assert frame_b[:4] == bytes([0x11, 0x22, 0x55, 0x66])
    assert frame_b[4:6] == bytes(2)


def test_pattern_compare() -> None:
    doc = json.loads(PATTERN.read_text())
    events = pattern_events(doc)
    rows, image, state = render(events)
    assert len(rows) == len(doc["rects"]), rows
    sha = hashlib.sha256(image).hexdigest()
    entry = entry_for(state, sha)
    header = "t_ps," + ",".join(HEADER) + "\n"
    lines = [f"{k},lcd1,{','.join(map(str, row))}\n" for k, row in enumerate(rows)]
    run_csv = header + "".join(lines)
    assert compare("lcd1", rows, image, state, run_csv, image, entry) == [], "the right pattern run passes"
    assert compare("lcd1", rows, image, state, header, image, entry), "a run with no row fails"
    assert compare("lcd1", rows, image, state, header + "".join(lines[:-1]), image, entry), "a missing row fails"
    assert compare("lcd1", rows, image, state, run_csv + lines[-1], image, entry), "an extra row fails"
    wider = "".join(line.rstrip("\n") + ",x\n" for line in (header, *lines))
    assert compare("lcd1", rows, image, state, wider, image, entry) == [], "a column past the known ones is ignored"
    assert compare("lcd1", rows, image, state, run_csv.replace(",hash", ",sha", 1), image, entry), "a lost column fails"
    blank = bytes(len(image))
    assert compare("lcd1", rows, image, state, run_csv, blank, entry_for(state, hashlib.sha256(blank).hexdigest())), (
        "a blank final image fails"
    )
    flat = {**doc, "rects": [{**doc["rects"][0], "fill": "bars", "colors": [0x1234, 0x1234]}]}
    try:
        pattern_events(flat)
    except ValueError:
        pass
    else:
        raise AssertionError("a rectangle of one value is refused")
    want = link_rows("u1", CS_PIN, events)
    link_header = ",".join(LINK_HEADER) + "\n"
    link = [f"{k},{k},{','.join(row)}\n" for k, row in enumerate(want)]
    assert compare_link(want, link_header + "".join(link)) == [], "the right link.csv passes"
    wider = "".join(line.rstrip("\n") + ",x\n" for line in (link_header, *link))
    assert compare_link(want, wider) == [], "a column past the known ones is ignored"
    assert compare_link(want, link_header.replace(",lanes", "") + "".join(link)), (
        "a link.csv with no lanes column fails"
    )
    assert compare_link(want, link_header), "a link.csv with no row fails"
    assert compare_link(want, link_header + "".join(link[:-1])), "a missing transfer fails"
    assert compare_link(want, link_header + "".join(link) + link[-1]), "an extra transfer fails"
    changed = link_header + "".join(link[:2]) + link[2].replace(",u1,", ",u9,", 1) + "".join(link[3:])
    assert "row 2 is " in compare_link(want, changed)[0], "a changed transfer is named by its row"
    relaned = link_header + "".join(link[:-1]) + link[-1].rsplit(",", 1)[0] + ",1\n"
    assert (want[0][-1], want[-1][-1]) == ("1", "4"), "a register write goes on one lane, colour on four"
    assert compare_link(want, relaned), "a transfer on other lanes fails"
