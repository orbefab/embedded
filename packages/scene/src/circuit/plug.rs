//! A scene's `on`: a part plugged into a board with holes, such as a breadboard. One pin's leg
//! goes into the hole `on` names, the part turns about Z by `rotate`, and every other leg must
//! land in a hole too. Each leg's pin then joins its hole, as a `contact` wire would.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::view::{Placement, placement};
use super::{Circuit, ROOT};
use crate::diagnostic::Code;
use crate::model::{Part, PinDef, Plug};
use crate::name::did_you_mean;
use crate::source::FileId;
use crate::syntax;

/// A leg further than this from every hole is in none: half the 2.54 mm pitch.
const REACH_MM: f64 = 1.27;
/// How far a leg may sit from its hole's centre once the part is centred on its holes, as a
/// lead flexes into a real board. A 6 mm switch's 6.5 by 4.5 mm legs fit a 7.62 by 5.08 mm span.
const FIT_MM: f64 = 1.0;
/// A leg points down when its axis does, once placed.
const DOWN: f64 = -0.5;

/// How a drawn leg joins the hole it is drawn in.
pub(super) enum Join {
    /// `on` seats the part on the drawn hole.
    Seat,
    /// A contact wire to this `board.hole`.
    Contact(String),
    /// The drawn hole is held and its strip has no free hole.
    Full,
}

/// A part the scene plugs in, where it sits, and the hole under each leg.
pub(super) struct Seated {
    pub(super) placement: Placement,
    /// When two of the part's pins sit in one strip, the hint that parts them.
    pub(super) parted: Option<String>,
    pub(super) contacts: Vec<Contact>,
}

/// One leg in one hole.
pub(super) struct Contact {
    pub(super) board: usize,
    pub(super) hole: String,
    pub(super) hole_key: usize,
    pub(super) pin_key: usize,
    /// The leg's own pin as a scene ref, such as `nano.j1.1`.
    pub(super) leaf: String,
    /// The pin as the plugged part names it, such as `nano.D1`.
    pub(super) shown: String,
    /// The strip the hole sits in, by its index in the board's `holes.strips`.
    pub(super) strip: Option<usize>,
}

struct Hole {
    name: String,
    key: usize,
    at: [f64; 3],
    /// The strip the hole sits in, by its index in the board's `holes.strips`.
    strip: Option<usize>,
}

struct Leg {
    /// In the plugged part's frame.
    tip: [f64; 3],
    pin_key: usize,
    lead: u64,
    leaf: String,
    shown: String,
}

impl Leg {
    /// The leg as a person names it: its pin, and the lead when its pin has more than one leg.
    fn named(&self, legs: &[Leg]) -> String {
        if legs.iter().filter(|l| l.shown == self.shown).count() > 1 {
            format!(
                "`{}` (lead {} of `{}`)",
                self.shown,
                self.lead,
                pin_owner(&self.leaf)
            )
        } else {
            format!("`{}`", self.shown)
        }
    }
}

/// An angle above -180 and at most 180 degrees.
fn half_turns(angle: f64) -> f64 {
    let turned = angle.rem_euclid(360.0);
    if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    }
}

/// The turns about Z a hint tries, as a layout writes them.
const QUARTERS: [f64; 4] = [0.0, 90.0, 180.0, -90.0];
/// How far from the named hole a hint looks for another: four pitches.
const NEARBY_MM: f64 = 10.16;

fn pin_owner(leaf: &str) -> &str {
    leaf.rsplit_once('.').map_or(leaf, |(owner, _)| owner)
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

fn unturn(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|c| m[0][c] * v[0] + m[1][c] * v[1] + m[2][c] * v[2])
}

/// The pin `on` names: the leg as the placed part shows it, without that part's name.
fn on_pin(part: &str, leg: &Leg) -> String {
    let prefix = format!("{part}.");
    leg.shown
        .strip_prefix(&prefix)
        .unwrap_or(&leg.shown)
        .to_string()
}

fn compose(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn flat(a: [f64; 3], b: [f64; 2]) -> f64 {
    libm::hypot(a[0] - b[0], a[1] - b[1])
}

/// Rounded to a micrometre, so a placement reads the same however its sums ran.
fn round(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

/// Where one leg lands.
enum Landing<'h> {
    /// Further than [`REACH_MM`] from every hole, at `at`.
    Missed {
        at: [f64; 3],
        nearest: &'h Hole,
        distance: f64,
    },
    /// Over `hole`, but further than [`FIT_MM`] from its centre once the part centres.
    Bent {
        hole: &'h Hole,
        distance: f64,
    },
    In(&'h Hole),
}

/// A part with one leg in one hole and turned about Z: where it sits on the board, and where each
/// of its legs lands, in the order of its legs.
struct Fit<'h> {
    shift: [f64; 2],
    landings: Vec<Landing<'h>>,
}

impl Fit<'_> {
    /// Whether every leg is in a hole, no two in one.
    fn lands(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.landings
            .iter()
            .all(|l| matches!(l, Landing::In(hole) if seen.insert(hole.key)))
    }

    /// Whether one strip holds legs of two of the part's pins.
    fn shorts(&self, legs: &[Leg]) -> bool {
        let mut pins: BTreeMap<usize, &str> = BTreeMap::new();
        legs.iter()
            .zip(&self.landings)
            .any(|(leg, landing)| match landing {
                Landing::In(Hole {
                    strip: Some(strip), ..
                }) => *pins.entry(*strip).or_insert(&leg.shown) != leg.shown,
                _ => false,
            })
    }
}

fn nearest(holes: &[Hole], at: [f64; 3]) -> (&Hole, f64) {
    holes
        .iter()
        .map(|h| (h, flat(at, [h.at[0], h.at[1]])))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a board with holes has one")
}

/// `named`'s leg in `hole` and the part turned `degrees` about Z on the board: each leg lands on
/// the nearest hole, then the part shifts by the mean of its legs' offsets, as leads flex.
fn fit<'h>(legs: &[Leg], named: &Leg, hole: &Hole, degrees: f64, holes: &'h [Hole]) -> Fit<'h> {
    let turned = rotation([0.0, 0.0, degrees]);
    let place = |leg: &Leg, shift: [f64; 2]| {
        let t = turn(&turned, leg.tip);
        [t[0] + shift[0], t[1] + shift[1], t[2]]
    };
    let tip = turn(&turned, named.tip);
    let shift = [hole.at[0] - tip[0], hole.at[1] - tip[1]];
    let first: Vec<([f64; 3], &Hole, f64)> = legs
        .iter()
        .map(|leg| {
            let at = place(leg, shift);
            let (hole, distance) = nearest(holes, at);
            (at, hole, distance)
        })
        .collect();
    if first.iter().any(|&(_, _, distance)| distance > REACH_MM) {
        let landings = first
            .into_iter()
            .map(|(at, nearest, distance)| match distance > REACH_MM {
                true => Landing::Missed {
                    at,
                    nearest,
                    distance,
                },
                false => Landing::In(nearest),
            })
            .collect();
        return Fit { shift, landings };
    }
    let count = legs.len() as f64;
    let shift = [0, 1]
        .map(|k| shift[k] + first.iter().map(|(at, h, _)| h.at[k] - at[k]).sum::<f64>() / count);
    let landings = legs
        .iter()
        .map(|leg| match nearest(holes, place(leg, shift)) {
            (hole, distance) if distance > FIT_MM => Landing::Bent { hole, distance },
            (hole, _) => Landing::In(hole),
        })
        .collect();
    Fit { shift, landings }
}

/// What a hint offers a part that misses its holes or shorts a strip, found by trying it.
enum Remedy {
    /// A quarter turn from the same hole.
    Turn(f64),
    /// Another hole near it, and a quarter turn.
    Elsewhere {
        hole: String,
        turn: f64,
    },
    Nothing,
}

/// The first quarter turn from `from` that seats every leg in a free hole with no strip joining
/// two pins, other than `now`, the turn that failed. Else the nearest hole within [`NEARBY_MM`]
/// with a quarter turn that does, trying `now` first.
fn remedy(
    legs: &[Leg],
    named: &Leg,
    from: &Hole,
    now: f64,
    holes: &[Hole],
    taken: &BTreeMap<usize, String>,
) -> Remedy {
    let now = half_turns(now);
    let seats = |hole: &Hole, degrees: f64| {
        let fit = fit(legs, named, hole, degrees, holes);
        fit.lands()
            && !fit.shorts(legs)
            && fit.landings.iter().all(|l| match l {
                Landing::In(hole) => !taken.contains_key(&hole.key),
                _ => false,
            })
    };
    let others = QUARTERS.into_iter().filter(|&t| (t - now).abs() > 1e-9);
    if let Some(turn) = others.clone().find(|&t| seats(from, t)) {
        return Remedy::Turn(turn);
    }
    let mut near: Vec<(&Hole, f64)> = holes
        .iter()
        .filter(|h| h.key != from.key)
        .map(|h| (h, flat(h.at, [from.at[0], from.at[1]])))
        .filter(|&(_, d)| d <= NEARBY_MM)
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    let turns: Vec<f64> = QUARTERS
        .into_iter()
        .filter(|&t| (t - now).abs() <= 1e-9)
        .chain(others)
        .collect();
    for (hole, _) in near {
        if let Some(&turn) = turns.iter().find(|&&t| seats(hole, t)) {
            return Remedy::Elsewhere {
                hole: hole.name.clone(),
                turn,
            };
        }
    }
    Remedy::Nothing
}

impl Remedy {
    fn hint(&self, part: &str, pin: &str, board: &str, from: &str) -> String {
        match self {
            Remedy::Turn(turn) => format!(
                "turn `{part}` with \"rotate\": [0, 0, {turn}], and every leg lands in a hole with no strip joining two of its pins"
            ),
            Remedy::Elsewhere { hole, turn } => format!(
                "no quarter turn seats `{part}` from {board}.{from}; plug it in as \"{part}\": {{ \"on\": {{ \"{pin}\": \"{board}.{hole}\" }}, \"rotate\": [0, 0, {turn}] }}"
            ),
            Remedy::Nothing => format!(
                "no quarter turn seats `{part}` from {board}.{from}, nor from a hole within {NEARBY_MM} mm of it; plug `{part}.{pin}` into a hole where `{part}`'s other legs land in holes too"
            ),
        }
    }
}

impl Circuit<'_> {
    /// Seats every part the scene plugs in, then joins each leg's pin to its hole.
    pub(super) fn plug(&mut self) {
        let project = self.project;
        let scene = &project.scene;
        let mut trees: BTreeMap<FileId, Value> = BTreeMap::new();
        let mut taken: BTreeMap<usize, String> = BTreeMap::new();
        for plug in &scene.plugs {
            let Some(seated) = self.seat(plug, &mut trees, &mut taken) else {
                continue;
            };
            let inst = self.insts[ROOT].children[&plug.name];
            self.seated.insert(inst, seated);
        }
        let joins: Vec<(usize, usize)> = self
            .seated
            .values()
            .flat_map(|s| s.contacts.iter().map(|c| (c.pin_key, c.hole_key)))
            .collect();
        for (a, b) in joins {
            self.union(a, b);
        }
    }

    fn tree(&self, trees: &mut BTreeMap<FileId, Value>, file: FileId) -> Value {
        trees
            .entry(file)
            .or_insert_with(|| {
                let mut quiet = crate::diagnostic::Findings::default();
                syntax::parse(&self.project.sources, file, &mut quiet)
                    .map_or(Value::Null, |node| node.to_value())
            })
            .clone()
    }

    fn seat(
        &mut self,
        plug: &Plug,
        trees: &mut BTreeMap<FileId, Value>,
        taken: &mut BTreeMap<usize, String>,
    ) -> Option<Seated> {
        let project = self.project;
        let inst = *self.insts[ROOT].children.get(&plug.name)?;
        self.part(inst)?;
        let board_name = &plug.hole.segs[0];
        let hole_name = &plug.hole.segs[1];
        let Some(&board) = self.insts[ROOT].children.get(board_name) else {
            let names = self.insts[ROOT].children.keys().map(String::as_str);
            let hint = did_you_mean(board_name, names, "");
            self.out.push(
                Code::UnknownRef,
                plug.hole.span,
                format!("no part named `{board_name}` to plug `{}` into", plug.name),
                format!("{hint}name a breadboard in \"parts\", as \"bb\": {{ \"type\": \"breadboard-400\" }}"),
            );
            return None;
        };
        let board_part = self.part(board)?;
        let Some(holes) = &board_part.holes else {
            self.out.push(
                Code::Layout,
                plug.hole.span,
                format!(
                    "`{board_name}` is a {}, which has no holes to plug into",
                    board_part.label
                ),
                "plug into a breadboard, or place the part with \"at\" and wire it".into(),
            );
            return None;
        };
        let scene_tree = self.tree(trees, project.scene.file);
        let written = &scene_tree["layout"][board_name.as_str()];
        if !written["on"].is_null() {
            self.out.push(
                Code::Layout,
                plug.hole.span,
                format!("`{board_name}` is itself plugged in, so nothing plugs into it"),
                format!("place `{board_name}` with \"at\""),
            );
            return None;
        }
        let Some(board_at) = placement(written) else {
            self.out.push(
                Code::Layout,
                plug.hole.span,
                format!(
                    "`{board_name}` has no position, so `{}` has nowhere to sit",
                    plug.name
                ),
                format!("place the board first, as \"{board_name}\": {{ \"at\": [0, 0, 0] }}"),
            );
            return None;
        };
        if board_at.rotate[0] != 0.0 || board_at.rotate[1] != 0.0 {
            self.out.push(
                Code::Layout,
                plug.hole.span,
                format!("`{board_name}` is tilted, and a part plugs into a board that turns about Z only"),
                format!("write \"rotate\": [0, 0, {}] on `{board_name}`", board_at.rotate[2]),
            );
            return None;
        }
        let board_holes = self.holes(board, board_part);
        let Some(first_hole) = board_holes.iter().find(|h| &h.name == hole_name) else {
            let hint = crate::holes::near_hole_hint(
                hole_name,
                board_holes.iter().map(|h| h.name.as_str()),
                &format!("{board_name}."),
            );
            self.out.push(
                Code::UnknownPin,
                plug.hole.span,
                format!("`{board_name}` has no hole `{hole_name}`"),
                format!(
                    "{hint}a {}'s holes are named row then column: {}",
                    board_part.label,
                    crate::holes::holes_text(board_holes.iter().map(|h| h.name.as_str()))
                ),
            );
            return None;
        };

        let legs = self.legs(inst, trees);
        let shown = format!("{}.{}", plug.name, plug.pin);
        let named: Vec<&Leg> = legs.iter().filter(|l| l.shown == shown).collect();
        let pin_keys: Vec<usize> = {
            let mut keys: Vec<usize> = named.iter().map(|l| l.pin_key).collect();
            keys.dedup();
            keys
        };
        let first = match (
            self.keys.get(&(inst, plug.pin.clone())),
            &named[..],
            pin_keys.len(),
        ) {
            (None, _, _) => {
                let part = self.part(inst)?;
                let hint = did_you_mean(&plug.pin, part.pins.iter().map(|p| p.name.as_str()), "");
                self.out.push(
                    Code::UnknownPin,
                    plug.pin_span,
                    format!("`{}` has no pin `{}`", plug.name, plug.pin),
                    format!("{hint}its pins: {}", part.pin_list()),
                );
                return None;
            }
            (Some(_), [], _) => {
                self.out.push(
                    Code::Layout,
                    plug.pin_span,
                    format!("`{shown}` has no leg that goes into a hole"),
                    "name a pin on a leg that points down, such as an LED's anode or a header pin"
                        .into(),
                );
                return None;
            }
            (Some(_), _, 2..) => {
                self.out.push(
                    Code::Layout,
                    plug.pin_span,
                    format!(
                        "`{shown}` is on {} legs, so it names no one hole",
                        named.len()
                    ),
                    "name a pin on one leg".into(),
                );
                return None;
            }
            (Some(_), [first, ..], _) => *first,
        };

        let fit = fit(&legs, first, first_hole, plug.turn, &board_holes);
        let remedy = || {
            remedy(&legs, first, first_hole, plug.turn, &board_holes, taken).hint(
                &plug.name,
                &plug.pin,
                board_name,
                &first_hole.name,
            )
        };
        let mut failed = false;
        let mut hint = None;
        for (leg, landing) in legs.iter().zip(&fit.landings) {
            let message = match *landing {
                Landing::Missed {
                    at,
                    nearest,
                    distance,
                } => self.missed(&board_holes, at, board_name, nearest, distance),
                Landing::Bent { hole, distance } => format!(
                    "lands {distance:.2} mm from the centre of {board_name}.{}, and a leg bends at most {FIT_MM} mm",
                    hole.name
                ),
                Landing::In(_) => continue,
            };
            let hint = hint.get_or_insert_with(remedy).clone();
            self.out.push(
                Code::Layout,
                plug.span,
                format!("{} {message}", leg.named(&legs)),
                hint,
            );
            failed = true;
        }
        if failed {
            return None;
        }
        let mut contacts = Vec::new();
        for (leg, landing) in legs.iter().zip(&fit.landings) {
            let Landing::In(hole) = *landing else {
                continue;
            };
            let who = leg.named(&legs);
            if let Some(other) = taken.get(&hole.key) {
                self.out.push(
                    Code::Layout,
                    plug.span,
                    format!(
                        "{who} lands in {board_name}.{}, which {other} already holds",
                        hole.name
                    ),
                    "a hole holds one leg; plug one of them into another hole".into(),
                );
                failed = true;
                continue;
            }
            contacts.push((
                hole.key,
                who,
                Contact {
                    board,
                    hole: hole.name.clone(),
                    hole_key: hole.key,
                    pin_key: leg.pin_key,
                    leaf: leg.leaf.clone(),
                    shown: leg.shown.clone(),
                    strip: hole.strip,
                },
            ));
        }
        if failed {
            return None;
        }
        let parted = fit.shorts(&legs).then(remedy);
        let shift = fit.shift;
        let deepest = legs.iter().map(|l| -l.tip[2]).fold(0.0, f64::max);
        let rise = (deepest - holes.depth).max(0.0);
        let local = [shift[0], shift[1], first_hole.at[2] + rise];
        let board_turn = rotation([0.0, 0.0, board_at.rotate[2]]);
        let moved = turn(&board_turn, local);
        let at = [0, 1, 2].map(|k| round(board_at.at[k] + moved[k]));
        Some(Seated {
            placement: Placement {
                at,
                rotate: [0.0, 0.0, round(half_turns(board_at.rotate[2] + plug.turn))],
            },
            parted,
            contacts: contacts
                .into_iter()
                .map(|(key, who, contact)| {
                    taken.insert(key, who);
                    contact
                })
                .collect(),
        })
    }

    /// Where a leg that is in no hole landed: in a gap between two rows of holes, such as the
    /// centre channel, or off the holes.
    fn missed(
        &self,
        holes: &[Hole],
        at: [f64; 3],
        board: &str,
        nearest: &Hole,
        distance: f64,
    ) -> String {
        let column = holes.iter().filter(|h| (h.at[0] - at[0]).abs() <= REACH_MM);
        let below = column
            .clone()
            .filter(|h| h.at[1] < at[1])
            .max_by(|a, b| a.at[1].total_cmp(&b.at[1]));
        let above = column
            .filter(|h| h.at[1] > at[1])
            .min_by(|a, b| a.at[1].total_cmp(&b.at[1]));
        match (below, above) {
            (Some(below), Some(above)) => format!(
                "lands between {board}.{} and {board}.{}, in the gap where `{board}` has no hole",
                below.name, above.name
            ),
            _ => format!(
                "lands {distance:.2} mm from the nearest hole, {board}.{}, off `{board}`'s holes",
                nearest.name
            ),
        }
    }

    /// Each hole of a board: its pin, and its mouth in the board's frame.
    fn holes(&self, board: usize, part: &Part) -> Vec<Hole> {
        let at: BTreeMap<u64, [f64; 3]> = self
            .anchors_of(part)
            .iter()
            .filter(|a| a["kind"] == "socket")
            .filter_map(|a| Some((a["lead"].as_u64()?, three(&a["at"])?)))
            .collect();
        let strips: BTreeMap<&str, usize> = part
            .holes
            .iter()
            .flat_map(|h| h.strips.iter().enumerate())
            .flat_map(|(index, strip)| strip.iter().map(move |hole| (hole.as_str(), index)))
            .collect();
        let tree = self.part_tree(part);
        part.pins
            .iter()
            .filter_map(|pin| {
                let lead = Self::lead_of(&tree, &pin.name)?;
                Some(Hole {
                    name: pin.name.clone(),
                    key: self.keys[&(board, pin.name.clone())],
                    at: *at.get(&lead)?,
                    strip: strips.get(pin.name.as_str()).copied(),
                })
            })
            .collect()
    }

    /// The one lead of a pin on one lead.
    fn lead_of(tree: &Value, pin: &str) -> Option<u64> {
        let leads = tree["pins"][pin].as_array()?;
        match leads[..] {
            [ref lead] => lead.as_u64(),
            _ => None,
        }
    }

    fn part_tree(&self, part: &Part) -> Value {
        let mut quiet = crate::diagnostic::Findings::default();
        syntax::parse(&self.project.sources, part.file, &mut quiet)
            .map_or(Value::Null, |n| n.to_value())
    }

    /// Every leg under `inst` that points down once placed, in `inst`'s frame: the leg anchors of
    /// its own part and of each child at any depth, placed by each parent's `layout`.
    fn legs(&self, inst: usize, trees: &mut BTreeMap<FileId, Value>) -> Vec<Leg> {
        let parents = self.parents();
        let mut legs = Vec::new();
        let identity = rotation([0.0; 3]);
        let mut stack = vec![(inst, identity, [0.0; 3])];
        while let Some((at, frame, origin)) = stack.pop() {
            let Some(part) = self.part(at) else { continue };
            let tree = self.tree(trees, part.file);
            for anchor in self.anchors_of(part) {
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
                if turn(&frame, axis)[2] > DOWN {
                    continue;
                }
                let Some(pin) = part.pins.iter().find(|p| {
                    matches!(p.def, PinDef::Leads)
                        && tree["pins"][&p.name]
                            .as_array()
                            .is_some_and(|l| l.iter().any(|n| n.as_u64() == Some(lead)))
                }) else {
                    continue;
                };
                let t = turn(&frame, tip);
                legs.push(Leg {
                    tip: [origin[0] + t[0], origin[1] + t[1], origin[2] + t[2]],
                    pin_key: self.keys[&(at, pin.name.clone())],
                    lead,
                    leaf: format!("{}.{}", self.insts[at].dotted(), pin.name),
                    shown: self
                        .hover(&parents, at, &pin.name)
                        .trim_start_matches('@')
                        .to_string(),
                });
            }
            for (name, &child) in &self.insts[at].children {
                let place = placement(&tree["layout"][name.as_str()]).unwrap_or(Placement {
                    at: [0.0; 3],
                    rotate: [0.0; 3],
                });
                let offset = turn(&frame, place.at);
                stack.push((
                    child,
                    compose(&frame, &rotation(place.rotate)),
                    [
                        origin[0] + offset[0],
                        origin[1] + offset[1],
                        origin[2] + offset[2],
                    ],
                ));
            }
        }
        legs.sort_by(|a, b| a.leaf.cmp(&b.leaf).then(a.lead.cmp(&b.lead)));
        legs
    }

    /// Holes a plugged leg or a scene wire already names.
    fn held_holes(&self) -> BTreeSet<usize> {
        let mut held = BTreeSet::new();
        for seated in self.seated.values() {
            for contact in &seated.contacts {
                held.insert(contact.hole_key);
            }
        }
        for wire in &self.project.scene.comp.wires {
            for end in &wire.ends {
                if let Ok(Some(key)) = self.lookup(ROOT, end)
                    && self.strip_of(key).is_some()
                {
                    held.insert(key);
                }
            }
        }
        held
    }

    /// A top part whose `at` puts a downward leg into a hole of a board that has its own `at`.
    /// The page draws that leg in the hole. The value is the board, the hole, the pin `on`
    /// would name, and how to join it. When that hole is held, the join is a contact wire to
    /// a free hole in the same strip. A board with no `at` is drawn in a row, so it is not
    /// under the part. `root` is each pin's net. A leg whose pin already shares the hole's net
    /// is skipped. Another leg of the same part, in a hole it does not share, is still unseated.
    pub(super) fn hovering(
        &self,
        root: &[usize],
    ) -> BTreeMap<String, (String, String, String, Join)> {
        let mut trees = BTreeMap::new();
        let scene = self.tree(&mut trees, self.project.scene.file);
        let layout = &scene["layout"];
        let tops: Vec<(String, usize)> = self.insts[ROOT]
            .children
            .iter()
            .map(|(name, id)| (name.clone(), *id))
            .collect();
        let mut boards = Vec::new();
        for (name, id) in &tops {
            let Some(part) = self.part(*id) else {
                continue;
            };
            if part.holes.is_none() {
                continue;
            }
            let written = &layout[name.as_str()];
            if !written["on"].is_null() {
                continue;
            }
            let Some(place) = placement(written) else {
                continue;
            };
            let holes = self.holes(*id, part);
            if !holes.is_empty() {
                let depth = part.holes.as_ref().map(|holes| holes.depth).unwrap_or(0.0);
                boards.push((name.clone(), place, holes, depth));
            }
        }
        let held = self.held_holes();
        let leads: BTreeMap<usize, String> = held
            .iter()
            .copied()
            .map(|key| (key, self.hole_label(key)))
            .collect();
        let mut found = BTreeMap::new();
        for (name, id) in &tops {
            let written = &layout[name.as_str()];
            if written["at"].is_null() || !written["on"].is_null() {
                continue;
            }
            let Some(place) = placement(written) else {
                continue;
            };
            let legs = self.legs(*id, &mut trees);
            let turned = rotation(place.rotate);
            let mut occupied = Vec::new();
            let mut best: Option<(f64, String, String, String, usize)> = None;
            for leg in &legs {
                let tip = turn(&turned, leg.tip);
                let world = [
                    place.at[0] + tip[0],
                    place.at[1] + tip[1],
                    place.at[2] + tip[2],
                ];
                for (board, board_at, holes, depth) in &boards {
                    if board == name {
                        continue;
                    }
                    let delta = [
                        world[0] - board_at.at[0],
                        world[1] - board_at.at[1],
                        world[2] - board_at.at[2],
                    ];
                    let local = unturn(&rotation(board_at.rotate), delta);
                    for hole in holes {
                        // A tip above the mouth, or further below it than the hole is deep,
                        // is not in the board.
                        if local[2] > hole.at[2] + 0.1 || local[2] < hole.at[2] - depth - 0.1 {
                            continue;
                        }
                        let distance = libm::hypot(local[0] - hole.at[0], local[1] - hole.at[1]);
                        if distance > REACH_MM {
                            continue;
                        }
                        occupied.push(hole.key);
                        if leg.pin_key < root.len()
                            && hole.key < root.len()
                            && root[leg.pin_key] == root[hole.key]
                        {
                            continue;
                        }
                        if best.as_ref().is_some_and(|&(near, ..)| distance >= near) {
                            continue;
                        }
                        best = Some((
                            distance,
                            board.clone(),
                            hole.name.clone(),
                            on_pin(name, leg),
                            hole.key,
                        ));
                    }
                }
            }
            if let Some((_, board, hole, pin, key)) = best {
                // `on` claims every hole this part is drawn in. A held one takes a contact
                // wire instead, to a free hole in that strip when the drawn hole itself is held.
                let join = if held.contains(&key) {
                    match self.free_hole(key, &leads) {
                        Some(target) => Join::Contact(target),
                        None => Join::Full,
                    }
                } else if occupied
                    .iter()
                    .any(|&other| other != key && held.contains(&other))
                {
                    Join::Contact(format!("{board}.{hole}"))
                } else {
                    Join::Seat
                };
                found.insert(name.clone(), (board, hole, pin, join));
            }
        }
        found
    }
}

fn three(value: &Value) -> Option<[f64; 3]> {
    let items = value.as_array()?;
    match items[..] {
        [ref x, ref y, ref z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}
