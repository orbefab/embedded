//! Logic levels of a transient run: `traces/logic.vcd`, and `observations.logic` on success.
//! Both are written from one `Logic`.

use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::name::{Name, Ref};
use crate::time::Time;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    High,
}

impl Level {
    pub fn from_bool(high: bool) -> Level {
        if high { Level::High } else { Level::Low }
    }

    fn bit(self) -> char {
        match self {
            Level::Low => '0',
            Level::High => '1',
        }
    }
}

/// Every logic signal's initial level and its changes, in time order.
#[derive(Clone, Debug, PartialEq)]
pub struct Logic {
    /// `Ref::Logic`, in ref order. A signal's index is its VCD identifier's index.
    signals: Vec<Ref>,
    initial: Vec<Level>,
    /// Each signal's level after the last change.
    now: Vec<Level>,
    /// Ascending `at`. Within one `at`, ascending `signal`, at most one change per signal.
    changes: Vec<Change>,
}

/// `signal` reads `level` from `at` on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    #[serde(rename = "t_ps")]
    pub at: Time,
    #[serde(skip)]
    pub signal: usize,
    pub level: Level,
}

impl Logic {
    /// Panics unless `signals` are `Ref::Logic` in ref order, one level each. Either is an
    /// engine bug.
    pub fn new(signals: Vec<Ref>, initial: Vec<Level>) -> Logic {
        assert!(signals.iter().all(|s| matches!(s, Ref::Logic(..))));
        assert!(signals.is_sorted());
        assert_eq!(signals.len(), initial.len());
        Logic {
            signals,
            now: initial.clone(),
            initial,
            changes: Vec::new(),
        }
    }

    /// `signal` reads `level` from `at` on. A level equal to the one in force is dropped. A
    /// change that restores the level before `at` removes the change at `at`, so a zero-time
    /// glitch leaves no edge. Panics if `at` is before the last change, which is an engine bug.
    pub fn set(&mut self, at: Time, signal: usize, level: Level) {
        assert!(self.changes.last().is_none_or(|last| last.at <= at));
        if level == self.now[signal] {
            return;
        }
        self.now[signal] = level;
        let same_at = self.changes.partition_point(|change| change.at < at);
        let run = &self.changes[same_at..];
        match run.binary_search_by_key(&signal, |change| change.signal) {
            // Levels are binary, so a second change at one `at` restores the first's level.
            Ok(k) => {
                self.changes.remove(same_at + k);
            }
            Err(k) => self
                .changes
                .insert(same_at + k, Change { at, signal, level }),
        }
    }

    pub fn signals(&self) -> &[Ref] {
        &self.signals
    }

    pub fn initial(&self) -> &[Level] {
        &self.initial
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Each signal's `$scope`, `$var` name and identifier, in signal order.
    pub(crate) fn vars(&self) -> impl Iterator<Item = (&Ref, &Name, &Name, String)> {
        self.signals.iter().enumerate().map(|(index, signal)| {
            let Ref::Logic(part, name) = signal else {
                unreachable!("Logic::new takes only logic refs");
            };
            (signal, part, name, identifier(index))
        })
    }

    /// `traces/logic.vcd`: `$timescale 1 ps`, one `$scope module <part>` per chip in ref order,
    /// `$dumpvars` at `#0`, then one `#<ps>` block per time with a change. Integers and fixed
    /// tokens only, with no `$date` and no `$version`.
    pub(crate) fn vcd(&self) -> Vec<u8> {
        let mut out = String::from("$timescale 1 ps $end\n");
        let mut scope: Option<&Name> = None;
        for (_, part, name, id) in self.vars() {
            if scope != Some(part) {
                if scope.is_some() {
                    out.push_str("$upscope $end\n");
                }
                out.push_str(&format!("$scope module {} $end\n", part.as_str()));
                scope = Some(part);
            }
            out.push_str(&format!("$var wire 1 {id} {} $end\n", name.as_str()));
        }
        if scope.is_some() {
            out.push_str("$upscope $end\n");
        }
        out.push_str("$enddefinitions $end\n#0\n$dumpvars\n");
        for (index, level) in self.initial.iter().enumerate() {
            out.push(level.bit());
            out.push_str(&identifier(index));
            out.push('\n');
        }
        out.push_str("$end\n");
        let mut last = None;
        for change in &self.changes {
            if last != Some(change.at) {
                out.push_str(&format!("#{}\n", change.at.as_ps()));
                last = Some(change.at);
            }
            out.push(change.level.bit());
            out.push_str(&identifier(change.signal));
            out.push('\n');
        }
        out.into_bytes()
    }
}

/// Base 94 over the printable characters from `!`, least significant first.
fn identifier(index: usize) -> String {
    let mut id = String::new();
    let mut rest = index;
    loop {
        id.push(char::from(b'!' + (rest % 94) as u8));
        rest /= 94;
        if rest == 0 {
            return id;
        }
    }
}

/// `{"#u1.logic.out": {"initial": "high", "edges": [{"t_ps": .., "level": ..}]}}`, keys in ref order.
impl Serialize for Logic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Signal<'a> {
            initial: Level,
            edges: Vec<&'a Change>,
        }
        let mut edges: BTreeMap<usize, Vec<&Change>> = BTreeMap::new();
        for change in &self.changes {
            edges.entry(change.signal).or_default().push(change);
        }
        let mut map = serializer.serialize_map(Some(self.signals.len()))?;
        for (index, (signal, initial)) in self.signals.iter().zip(&self.initial).enumerate() {
            let entry = Signal {
                initial: *initial,
                edges: edges.remove(&index).unwrap_or_default(),
            };
            map.serialize_entry(signal, &entry)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logic(signals: &[&str]) -> Logic {
        let refs = signals.iter().map(|s| s.parse().unwrap()).collect();
        Logic::new(refs, vec![Level::Low; signals.len()])
    }

    fn at(ps: u64) -> Time {
        Time::from_ps(ps)
    }

    #[test]
    fn a_level_already_in_force_is_no_edge() {
        let mut logic = logic(&["#u1.logic.out"]);
        logic.set(at(5), 0, Level::Low);
        assert!(logic.changes().is_empty());
        logic.set(at(5), 0, Level::High);
        logic.set(at(9), 0, Level::High);
        assert_eq!(
            logic.changes(),
            [Change {
                at: at(5),
                signal: 0,
                level: Level::High
            }]
        );
    }

    #[test]
    fn a_zero_time_glitch_leaves_no_edge() {
        let mut logic = logic(&["#u1.logic.out", "#u1.logic.trig"]);
        logic.set(at(3), 0, Level::High);
        logic.set(at(7), 1, Level::High);
        logic.set(at(7), 0, Level::Low);
        logic.set(at(7), 1, Level::Low);
        logic.set(at(7), 0, Level::High);
        assert_eq!(
            logic.changes(),
            [Change {
                at: at(3),
                signal: 0,
                level: Level::High
            }]
        );
        logic.set(at(7), 1, Level::High);
        let signals: Vec<_> = logic.changes().iter().map(|c| (c.at, c.signal)).collect();
        assert_eq!(signals, [(at(3), 0), (at(7), 1)]);
    }

    #[test]
    fn changes_at_one_time_keep_signal_order() {
        let mut logic = logic(&["#u1.logic.out", "#u1.logic.reset", "#u1.logic.trig"]);
        logic.set(at(4), 2, Level::High);
        logic.set(at(4), 0, Level::High);
        logic.set(at(4), 1, Level::High);
        let signals: Vec<_> = logic.changes().iter().map(|c| c.signal).collect();
        assert_eq!(signals, [0, 1, 2]);
    }

    #[test]
    #[should_panic]
    fn a_change_before_the_last_one_is_an_engine_bug() {
        let mut logic = logic(&["#u1.logic.out"]);
        logic.set(at(5), 0, Level::High);
        logic.set(at(4), 0, Level::Low);
    }

    #[test]
    fn identifiers_are_base_94_from_the_exclamation_mark() {
        assert_eq!(identifier(0), "!");
        assert_eq!(identifier(3), "$");
        assert_eq!(identifier(93), "~");
        assert_eq!(identifier(94), "!\"");
        assert_eq!(identifier(94 * 94 + 1), "\"!\"");
    }

    fn two_chips() -> Logic {
        let mut logic = Logic::new(
            ["#u1.logic.out", "#u1.logic.trig", "#u2.logic.out"]
                .map(|s| s.parse().unwrap())
                .to_vec(),
            vec![Level::High, Level::Low, Level::Low],
        );
        logic.set(at(231), 1, Level::High);
        logic.set(at(626), 0, Level::Low);
        logic.set(at(626), 2, Level::High);
        logic
    }

    #[test]
    fn the_vcd_scopes_each_chip_and_writes_integers_only() {
        let text = String::from_utf8(two_chips().vcd()).unwrap();
        assert_eq!(
            text,
            "$timescale 1 ps $end
$scope module u1 $end
$var wire 1 ! out $end
$var wire 1 \" trig $end
$upscope $end
$scope module u2 $end
$var wire 1 # out $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
1!
0\"
0#
$end
#231
1\"
#626
0!
1#
"
        );
    }

    #[test]
    fn observations_list_every_signal_in_ref_order_with_its_edges() {
        let text = serde_json::to_string(&two_chips()).unwrap();
        assert_eq!(
            text,
            r##"{"#u1.logic.out":{"initial":"high","edges":[{"t_ps":626,"level":"low"}]},"#u1.logic.trig":{"initial":"low","edges":[{"t_ps":231,"level":"high"}]},"#u2.logic.out":{"initial":"low","edges":[{"t_ps":626,"level":"high"}]}}"##
        );
    }
}
