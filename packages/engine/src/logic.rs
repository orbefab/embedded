//! Logic levels of a transient run: every change in `traces/logic.vcd`, and one summary per
//! signal in `observations.logic`. Both are written from one `Logic`.

use std::collections::BTreeMap;

use serde::Serialize;

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

/// Every logic signal's initial level and its changes, in time order. It has no `Serialize`:
/// observations hold its summary.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: Time,
    pub signal: usize,
    pub level: Level,
}

/// `observations.logic`: `{ref: {initial, final, count, first_ps, last_ps}}`, keys in ref order.
#[derive(Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LogicSummary(pub BTreeMap<Ref, SignalSummary>);

/// One signal over a run. Levels are binary, so the first change is to the level opposite
/// `initial`, and the last is to `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SignalSummary {
    pub initial: Level,
    /// The level after the last change.
    #[serde(rename = "final")]
    pub end: Level,
    pub count: usize,
    /// `None` when `count` is 0.
    #[serde(rename = "first_ps")]
    pub first: Option<Time>,
    #[serde(rename = "last_ps")]
    pub last: Option<Time>,
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

    /// One entry per signal: its initial and final level, how many changes, and when the
    /// first and the last were.
    pub fn summary(&self) -> LogicSummary {
        let mut summaries: Vec<SignalSummary> = self
            .initial
            .iter()
            .zip(&self.now)
            .map(|(&initial, &end)| SignalSummary {
                initial,
                end,
                count: 0,
                first: None,
                last: None,
            })
            .collect();
        for change in &self.changes {
            let summary = &mut summaries[change.signal];
            summary.count += 1;
            summary.first.get_or_insert(change.at);
            summary.last = Some(change.at);
        }
        LogicSummary(self.signals.iter().cloned().zip(summaries).collect())
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
    fn the_summary_counts_each_signals_changes_in_ref_order() {
        let text = serde_json::to_string(&two_chips().summary()).unwrap();
        assert_eq!(
            text,
            r##"{"#u1.logic.out":{"initial":"high","final":"low","count":1,"first_ps":626,"last_ps":626},"#u1.logic.trig":{"initial":"low","final":"high","count":1,"first_ps":231,"last_ps":231},"#u2.logic.out":{"initial":"low","final":"high","count":1,"first_ps":626,"last_ps":626}}"##
        );
    }

    #[test]
    fn a_cancelled_glitch_leaves_the_summary_and_a_quiet_signal_has_no_times() {
        let mut logic = logic(&["#u1.logic.out", "#u1.logic.reset"]);
        logic.set(at(3), 0, Level::High);
        logic.set(at(5), 0, Level::Low);
        logic.set(at(9), 0, Level::High);
        logic.set(at(9), 0, Level::Low);
        let summary = logic.summary();
        let [out, reset] =
            ["#u1.logic.out", "#u1.logic.reset"].map(|s| summary.0[&s.parse().unwrap()]);
        assert_eq!(
            out,
            SignalSummary {
                initial: Level::Low,
                end: Level::Low,
                count: 2,
                first: Some(at(3)),
                last: Some(at(5)),
            }
        );
        assert_eq!(
            reset,
            SignalSummary {
                initial: Level::Low,
                end: Level::Low,
                count: 0,
                first: None,
                last: None,
            }
        );
        assert_eq!(
            serde_json::to_value(reset).unwrap(),
            serde_json::json!({"initial": "low", "final": "low", "count": 0, "first_ps": null, "last_ps": null})
        );
    }
}
