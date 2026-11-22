//! How a catalog part sits in a breadboard, from its anchors and the hole pitch. `orbe part`
//! prints one line of it. The fit is the scene reader's: one named leg in one hole, a quarter
//! turn about Z, then a shift by the mean of the legs' offsets.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// A part's `part.jsonc` and its anchors, as `orbe part` already fetched them.
#[derive(Clone)]
pub struct PlugPart {
    pub value: Value,
    pub anchors: Vec<Value>,
}

/// A leg further than this from every hole is in none: half the 2.54 mm pitch.
const REACH_MM: f64 = 1.27;
/// How far a leg may sit from its hole's centre once the part is centred on its holes.
const FIT_MM: f64 = 1.0;
/// A leg points down when its axis does, once placed.
const DOWN: f64 = -0.5;
const PITCH_MM: f64 = 2.54;
/// The turns about Z a fit tries, in the order a hint tries them.
const QUARTERS: [f64; 4] = [0.0, 90.0, 180.0, -90.0];

struct Leg {
    tip: [f64; 3],
    /// `""` for the part's own lead, else the child instance, such as `j2`.
    leaf: String,
    lead: u64,
    /// The pin a scene names, such as `a` or `D13`.
    pin: String,
}

struct Hole {
    name: String,
    at: [f64; 2],
    strip: Option<usize>,
    column: u32,
    row: String,
}

struct Board {
    holes: Vec<Hole>,
    grid: BTreeMap<(i32, i32), Vec<usize>>,
}

struct Choice {
    pin: String,
    /// The lowest lead of `pin`, which is the leg `on` seats.
    leg: usize,
    /// One downward leg, so it is tried before a pin that has several.
    preferred: bool,
}

/// One line on how `id` plugs into a breadboard, or `None` when `load` has no such part.
/// `load` returns the part and the boards the line tries, including each immediate child.
pub fn breadboard_line(id: &str, load: &dyn Fn(&str) -> Option<PlugPart>) -> Option<String> {
    let value = part_value(id, load)?;
    if value.get("holes").is_some_and(Value::is_object) {
        return Some("it is a breadboard; other parts plug into its holes".into());
    }
    let legs = legs_of(id, &value, load);
    if legs.is_empty() {
        return Some("does not plug into a breadboard: it has no leads that go into a hole".into());
    }
    let choices = choices(&legs);
    let mut short = false;
    if choices.is_empty() {
        return Some(
            "does not plug into a breadboard: its leads do not land in the holes of one breadboard"
                .into(),
        );
    }
    for board_id in ["breadboard-400", "breadboard-830"] {
        let Some(board) = board(board_id, load) else {
            continue;
        };
        let holes = hole_order(&board);
        for &degrees in &QUARTERS {
            for &hole in &holes {
                for choice in &choices {
                    match seat(&legs, choice.leg, hole, degrees, &board) {
                        Some(Seat::Placed(landed)) => {
                            return Some(sentence(board_id, choice, &legs, &landed, &board));
                        }
                        Some(Seat::Short) => short = true,
                        None => {}
                    }
                }
            }
        }
    }
    Some(if short {
        "does not plug into a breadboard: two legs in one strip short".into()
    } else {
        "does not plug into a breadboard: its leads do not land in the holes of one breadboard"
            .into()
    })
}

fn sentence(
    board_id: &str,
    choice: &Choice,
    legs: &[Leg],
    landed: &[usize],
    board: &Board,
) -> String {
    let hole = &board.holes[landed[choice.leg]].name;
    let example = format!("\"on\": {{ \"{}\": \"bb.{hole}\" }}", choice.pin);
    let wide = if board_id == "breadboard-830" {
        ", only a breadboard-830 is wide enough"
    } else {
        ""
    };
    if legs.len() == 2
        && let Some(along) = along_one_row(choice.leg, landed, board)
    {
        let rest = if along == 1 {
            "and orbe seats the other lead in the next hole".to_string()
        } else {
            format!(
                "and orbe seats the other lead {along} holes along the row, so the part spans {} holes",
                along + 1
            )
        };
        return format!(
            "plugs into a breadboard: name one lead's hole with on, such as {example}, {rest}{wide}"
        );
    }
    format!(
        "plugs into a breadboard: name one pin's hole with on, such as {example}, and orbe seats the other leads{wide}"
    )
}

/// The other leg's column distance when both legs land on one row.
fn along_one_row(named: usize, landed: &[usize], board: &Board) -> Option<u32> {
    let here = &board.holes[landed[named]];
    let other = landed
        .iter()
        .enumerate()
        .find_map(|(leg, &hole)| (leg != named).then_some(&board.holes[hole]))?;
    let along = here.column.abs_diff(other.column);
    (here.row == other.row && along > 0).then_some(along)
}

fn choices(legs: &[Leg]) -> Vec<Choice> {
    let mut by_pin: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, leg) in legs.iter().enumerate() {
        by_pin.entry(&leg.pin).or_default().push(index);
    }
    let mut choices = Vec::new();
    for (pin, indexes) in by_pin {
        let leaves: BTreeSet<_> = indexes.iter().map(|&i| legs[i].leaf.as_str()).collect();
        if leaves.len() != 1 {
            continue;
        }
        let leg = *indexes
            .iter()
            .min_by_key(|&&i| legs[i].lead)
            .expect("a pin in the map has a leg");
        choices.push(Choice {
            pin: pin.to_string(),
            leg,
            preferred: indexes.len() == 1,
        });
    }
    choices.sort_by(|a, b| {
        b.preferred
            .cmp(&a.preferred)
            .then(legs[a.leg].lead.cmp(&legs[b.leg].lead))
            .then(a.pin.cmp(&b.pin))
    });
    choices
}

/// `c21` first, then the other holes by column and row. The first fit in this order is the one
/// `orbe part` names.
fn hole_order(board: &Board) -> Vec<usize> {
    let mut rest: Vec<usize> = (0..board.holes.len()).collect();
    rest.sort_by(|a, b| {
        let (a, b) = (&board.holes[*a], &board.holes[*b]);
        a.column
            .cmp(&b.column)
            .then(a.row.cmp(&b.row))
            .then(a.name.cmp(&b.name))
    });
    let mut order = Vec::with_capacity(rest.len());
    if let Some(c21) = rest.iter().position(|&i| board.holes[i].name == "c21") {
        order.push(rest[c21]);
        rest.remove(c21);
    }
    order.extend(rest);
    order
}

enum Seat {
    Placed(Vec<usize>),
    /// Every leg is within reach of a hole, and two pins share a strip or a hole.
    Short,
}

/// Each leg's hole when every leg lands in its own hole and no strip joins two pins.
/// [`Seat::Short`] when every leg is within reach and two pins share a strip or a hole.
/// `None` when the seat misses.
fn seat(legs: &[Leg], named: usize, hole: usize, degrees: f64, board: &Board) -> Option<Seat> {
    let named_leg = &legs[named];
    let hole = &board.holes[hole];
    let turned = rotation([0.0, 0.0, degrees]);
    let place = |leg: &Leg, shift: [f64; 2]| {
        let t = turn(&turned, leg.tip);
        [t[0] + shift[0], t[1] + shift[1]]
    };
    let tip = turn(&turned, named_leg.tip);
    let mut shift = [hole.at[0] - tip[0], hole.at[1] - tip[1]];
    let first: Vec<([f64; 2], usize, f64)> = legs
        .iter()
        .map(|leg| {
            let at = place(leg, shift);
            let (index, distance) = nearest(board, at)?;
            Some((at, index, distance))
        })
        .collect::<Option<_>>()?;
    if first.iter().any(|&(_, _, distance)| distance > REACH_MM) {
        return None;
    }
    let count = legs.len() as f64;
    shift = [0, 1].map(|k| {
        shift[k]
            + first
                .iter()
                .map(|(at, index, _)| board.holes[*index].at[k] - at[k])
                .sum::<f64>()
                / count
    });
    let mut seen = BTreeSet::new();
    let mut strips: BTreeMap<usize, &str> = BTreeMap::new();
    let mut landed = Vec::with_capacity(legs.len());
    let mut over_fit = false;
    let mut shorted = false;
    for leg in legs {
        let (index, distance) = nearest(board, place(leg, shift))?;
        if distance > REACH_MM {
            return None;
        }
        if distance > FIT_MM {
            over_fit = true;
        }
        if !seen.insert(index) {
            shorted = true;
        }
        let hole = &board.holes[index];
        if let Some(strip) = hole.strip
            && strips
                .insert(strip, leg.pin.as_str())
                .is_some_and(|other| other != leg.pin)
        {
            shorted = true;
        }
        landed.push(index);
    }
    if !over_fit && !shorted {
        Some(Seat::Placed(landed))
    } else if shorted {
        Some(Seat::Short)
    } else {
        None
    }
}

fn nearest(board: &Board, at: [f64; 2]) -> Option<(usize, f64)> {
    let cell = |v: f64| (v / PITCH_MM).floor() as i32;
    let (ix, iy) = (cell(at[0]), cell(at[1]));
    let mut best: Option<(usize, f64)> = None;
    for dx in -2..=2 {
        for dy in -2..=2 {
            let Some(indexes) = board.grid.get(&(ix + dx, iy + dy)) else {
                continue;
            };
            for &index in indexes {
                let hole = &board.holes[index];
                let distance = libm::hypot(at[0] - hole.at[0], at[1] - hole.at[1]);
                if best.is_none_or(|(other, so_far)| {
                    distance < so_far || (distance == so_far && index < other)
                }) {
                    best = Some((index, distance));
                }
            }
        }
    }
    best
}

fn board(id: &str, load: &dyn Fn(&str) -> Option<PlugPart>) -> Option<Board> {
    let value = part_value(id, load)?;
    let anchors = anchors(id, load);
    let at: BTreeMap<u64, [f64; 2]> = anchors
        .iter()
        .filter(|a| a["kind"] == "socket")
        .filter_map(|a| Some((a["lead"].as_u64()?, two(&a["at"])?)))
        .collect();
    let strips: BTreeMap<&str, usize> = value["holes"]["strips"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .flat_map(|(index, strip)| {
            strip
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |hole| Some((hole.as_str()?, index)))
        })
        .collect();
    let mut holes = Vec::new();
    let pins = value["pins"].as_object()?;
    for (name, leads) in pins {
        let Some(lead) = one_lead(leads) else {
            continue;
        };
        let Some(at) = at.get(&lead) else {
            continue;
        };
        let Some((row, column)) = row_column(name) else {
            continue;
        };
        holes.push(Hole {
            strip: strips.get(name.as_str()).copied(),
            name: name.clone(),
            at: *at,
            column,
            row,
        });
    }
    if holes.is_empty() {
        return None;
    }
    let mut grid: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for (index, hole) in holes.iter().enumerate() {
        let cell = |v: f64| (v / PITCH_MM).floor() as i32;
        grid.entry((cell(hole.at[0]), cell(hole.at[1])))
            .or_default()
            .push(index);
    }
    Some(Board { holes, grid })
}

fn row_column(name: &str) -> Option<(String, u32)> {
    let split = name.find(|c: char| c.is_ascii_digit())?;
    let (row, column) = name.split_at(split);
    if row.is_empty() {
        return None;
    }
    Some((row.to_string(), column.parse().ok()?))
}

fn legs_of(id: &str, value: &Value, load: &dyn Fn(&str) -> Option<PlugPart>) -> Vec<Leg> {
    let exposed = exposed(value, load);
    let mut legs = Vec::new();
    push_legs(
        &mut legs,
        id,
        "",
        &rotation([0.0; 3]),
        [0.0; 3],
        &exposed,
        load,
    );
    if let Some(parts) = value["parts"].as_object() {
        for (name, spec) in parts {
            if !exposed.children.contains(name) {
                continue;
            }
            let Some(ty) = spec["type"].as_str() else {
                continue;
            };
            let (at, rotate) = placement(&value["layout"][name]);
            let frame = rotation(rotate);
            push_legs(&mut legs, ty, name, &frame, at, &exposed, load);
        }
    }
    legs
}

/// A root pin's lead: own leads under `""`, and each child lead a root pin names.
struct Exposed {
    by_lead: BTreeMap<(String, u64), String>,
    children: BTreeSet<String>,
}

fn exposed(value: &Value, load: &dyn Fn(&str) -> Option<PlugPart>) -> Exposed {
    let mut out = Exposed {
        by_lead: BTreeMap::new(),
        children: BTreeSet::new(),
    };
    let Some(pins) = value["pins"].as_object() else {
        return out;
    };
    let mut labels: Vec<(String, String, String)> = Vec::new();
    for (pin, spec) in pins {
        if let Some(leads) = lead_list(spec) {
            for lead in leads {
                out.by_lead.insert((String::new(), lead), pin.clone());
            }
            continue;
        }
        for label in label_list(spec) {
            let Some((child, child_pin)) = label.rsplit_once('.') else {
                continue;
            };
            out.children.insert(child.to_string());
            labels.push((child.to_string(), child_pin.to_string(), pin.clone()));
        }
    }
    let mut child_leads: BTreeMap<String, BTreeMap<String, Vec<u64>>> = BTreeMap::new();
    for (child, child_pin, pin) in labels {
        let leads = child_leads.entry(child.clone()).or_insert_with(|| {
            part_value(child_type(value, &child), load)
                .and_then(|part| part["pins"].as_object().cloned())
                .map(|pins| {
                    pins.iter()
                        .filter_map(|(name, spec)| Some((name.clone(), lead_list(spec)?)))
                        .collect()
                })
                .unwrap_or_default()
        });
        let Some(leads) = leads.get(&child_pin) else {
            continue;
        };
        for &lead in leads {
            out.by_lead.insert((child.clone(), lead), pin.clone());
        }
    }
    out
}

fn child_type<'a>(value: &'a Value, child: &str) -> &'a str {
    value["parts"][child]["type"].as_str().unwrap_or("")
}

fn push_legs(
    legs: &mut Vec<Leg>,
    id: &str,
    leaf: &str,
    frame: &[[f64; 3]; 3],
    origin: [f64; 3],
    exposed: &Exposed,
    load: &dyn Fn(&str) -> Option<PlugPart>,
) {
    for anchor in anchors(id, load) {
        if anchor["kind"] != "leg" {
            continue;
        }
        let (Some(lead), Some(tip), Some(axis)) = (
            anchor["lead"].as_u64(),
            three(&anchor["at"]),
            three(&anchor["axis"]),
        ) else {
            continue;
        };
        if turn(frame, axis)[2] > DOWN {
            continue;
        }
        let Some(pin) = exposed.by_lead.get(&(leaf.to_string(), lead)) else {
            continue;
        };
        let t = turn(frame, tip);
        legs.push(Leg {
            tip: [origin[0] + t[0], origin[1] + t[1], origin[2] + t[2]],
            leaf: leaf.to_string(),
            lead,
            pin: pin.clone(),
        });
    }
}

fn lead_list(spec: &Value) -> Option<Vec<u64>> {
    let items = spec.as_array()?;
    items.iter().map(Value::as_u64).collect()
}

fn one_lead(spec: &Value) -> Option<u64> {
    match lead_list(spec)?.as_slice() {
        [lead] => Some(*lead),
        _ => None,
    }
}

fn label_list(spec: &Value) -> Vec<String> {
    match spec {
        Value::String(label) => vec![label.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn part_value(id: &str, load: &dyn Fn(&str) -> Option<PlugPart>) -> Option<Value> {
    load(id).map(|part| part.value)
}

fn anchors(id: &str, load: &dyn Fn(&str) -> Option<PlugPart>) -> Vec<Value> {
    load(id).map(|part| part.anchors).unwrap_or_default()
}

fn placement(value: &Value) -> ([f64; 3], [f64; 3]) {
    let at = three(&value["at"]).unwrap_or([0.0; 3]);
    let rotate = match &value["rotate"] {
        Value::Null => [0.0; 3],
        rotate => three(rotate).unwrap_or([0.0; 3]),
    };
    (at, rotate)
}

fn three(value: &Value) -> Option<[f64; 3]> {
    let items = value.as_array()?;
    match items[..] {
        [ref x, ref y, ref z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}

fn two(value: &Value) -> Option<[f64; 2]> {
    let [x, y, _] = three(value)?;
    Some([x, y])
}

/// The rotation about fixed X, then Y, then Z, in degrees, as a matrix.
fn rotation([rx, ry, rz]: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = libm::sincos(rx.to_radians());
    let (sy, cy) = libm::sincos(ry.to_radians());
    let (sz, cz) = libm::sincos(rz.to_radians());
    [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ]
}

fn turn(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}
