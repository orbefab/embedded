//! A scene's `on`: a part plugged into a board with holes, such as a breadboard. One pin's leg
//! goes into the hole `on` names, the part turns about Z by `rotate`, and every other leg must
//! land in a hole too. Each leg's pin then joins its hole, as a `contact` wire would.

use std::collections::BTreeMap;

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

/// A part the scene plugs in, where it sits, and the hole under each leg.
pub(super) struct Seated {
    pub(super) placement: Placement,
    /// The turn about Z on the board, in degrees, as the scene wrote it.
    pub(super) turn: f64,
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

/// The next quarter turn from `turn`, as a layout writes it.
pub(super) fn quarter(turn: f64) -> f64 {
    half_turns(turn + 90.0)
}

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
            let names = board_holes.iter().map(|h| h.name.as_str());
            let hint = did_you_mean(hole_name, names, &format!("{board_name}."));
            self.out.push(
                Code::UnknownPin,
                plug.hole.span,
                format!("`{board_name}` has no hole `{hole_name}`"),
                format!(
                    "{hint}a {}'s holes are named by row and column, from {} to {}",
                    board_part.label,
                    board_holes.first().map_or("", |h| h.name.as_str()),
                    board_holes.last().map_or("", |h| h.name.as_str())
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

        let turned = rotation([0.0, 0.0, plug.turn]);
        let tip0 = turn(&turned, first.tip);
        let mut shift = [first_hole.at[0] - tip0[0], first_hole.at[1] - tip0[1]];
        let nearest = |at: [f64; 3]| -> (&Hole, f64) {
            board_holes
                .iter()
                .map(|h| (h, flat(at, [h.at[0], h.at[1]])))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .expect("a board with holes has one")
        };
        let place = |leg: &Leg, shift: [f64; 2]| {
            let t = turn(&turned, leg.tip);
            [t[0] + shift[0], t[1] + shift[1], t[2]]
        };
        let mut off = [0.0, 0.0];
        let mut missed = false;
        for leg in &legs {
            let at = place(leg, shift);
            let (hole, distance) = nearest(at);
            if distance > REACH_MM {
                let message = self.missed(&board_holes, at, board_name, hole, distance);
                self.out.push(
                    Code::Layout,
                    plug.span,
                    format!("{} {message}", leg.named(&legs)),
                    format!(
                        "plug `{shown}` into another hole, or turn `{}` with \"rotate\": [0, 0, {}], so every leg lands in a hole",
                        plug.name,
                        quarter(plug.turn)
                    ),
                );
                missed = true;
                continue;
            }
            off[0] += hole.at[0] - at[0];
            off[1] += hole.at[1] - at[1];
        }
        if missed {
            return None;
        }
        let count = legs.len() as f64;
        shift = [shift[0] + off[0] / count, shift[1] + off[1] / count];
        let mut contacts = Vec::new();
        let mut strips: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, strip) in holes.strips.iter().enumerate() {
            for hole in strip {
                strips.insert(hole, index);
            }
        }
        for leg in &legs {
            let at = place(leg, shift);
            let (hole, distance) = nearest(at);
            if distance > FIT_MM {
                self.out.push(
                    Code::Layout,
                    plug.span,
                    format!("{} lands {distance:.2} mm from the centre of {board_name}.{}, and a leg bends at most {FIT_MM} mm", leg.named(&legs), hole.name),
                    format!("`{}`'s legs do not fit `{board_name}`'s 2.54 mm grid this way; turn it, or plug it in where its legs fit", plug.name),
                );
                missed = true;
                continue;
            }
            let who = leg.named(&legs);
            if let Some(other) = taken.get(&hole.key) {
                self.out.push(
                    Code::Layout,
                    plug.span,
                    format!(
                        "{} lands in {board_name}.{}, which {other} already holds",
                        leg.named(&legs),
                        hole.name
                    ),
                    "a hole holds one leg; plug one of them into another hole".into(),
                );
                missed = true;
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
                    strip: strips.get(hole.name.as_str()).copied(),
                },
            ));
        }
        if missed {
            return None;
        }
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
            turn: plug.turn,
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
        part.pins
            .iter()
            .filter_map(|pin| {
                let lead = self.lead_of(part, &pin.name)?;
                Some(Hole {
                    name: pin.name.clone(),
                    key: self.keys[&(board, pin.name.clone())],
                    at: *at.get(&lead)?,
                })
            })
            .collect()
    }

    /// The one lead of a pin on one lead.
    fn lead_of(&self, part: &Part, pin: &str) -> Option<u64> {
        let tree = self.part_tree(part);
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
}

fn three(value: &Value) -> Option<[f64; 3]> {
    let items = value.as_array()?;
    match items[..] {
        [ref x, ref y, ref z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}
