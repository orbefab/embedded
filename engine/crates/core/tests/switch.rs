mod common;

use common::{errors, with_parts};
use sim_core::netlist::{self, Contact, Model};
use sim_core::time::Time;

fn switch(events: &str, bounce: &str) -> String {
    with_parts(
        r#"{ "op": {} }"#,
        &format!(
            r#"{{
          "name": "sw1",
          "kind": "switch",
          "pins": {{ "a": "a", "b": "gnd" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open",
          "events": {events},
          "bounce": {bounce}
        }}"#
        ),
    )
}

fn bounce(window: &str, max_bounces: &str) -> String {
    format!(
        r#"{{
          "window": {{ "value": {window}, "unit": "ps", "source": "user" }},
          "max_bounces": {{ "value": {max_bounces}, "unit": "1", "source": "user" }}
        }}"#
    )
}

#[test]
fn a_switch_resolves_its_events_and_bounce_into_one_schedule() {
    let json = switch(
        r#"[{ "t_ps": 2000000000, "state": "closed" }, { "t_ps": 12000000000, "state": "open" }]"#,
        &bounce("1000000000", "4"),
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let (_, sw1) = netlist.parts().next().unwrap();
    let Model::Switch(switch) = &sw1.model else {
        panic!("{:?}", sw1.model);
    };
    assert_eq!(switch.events.len(), 2);
    let schedule = &switch.schedule;
    assert_eq!(schedule.initial(), Contact::Open);
    assert_eq!(schedule.edges().len(), 14);
    assert_eq!(
        schedule.edges(),
        netlist::bounce::expand(7, &sw1.name, &switch.events, switch.bounce.as_ref())
    );
    assert_eq!(schedule.before(Time::ZERO), Contact::Open);
    assert_eq!(schedule.after(Time::ZERO), Contact::Open);
    assert_eq!(
        schedule.after(Time::from_ps(2_000_000_000)),
        Contact::Closed
    );
    assert_eq!(schedule.before(Time::from_ps(2_000_000_000)), Contact::Open);
    assert_eq!(
        schedule.before(Time::from_ps(2_000_000_001)),
        Contact::Closed
    );
    assert_eq!(
        schedule.before(Time::from_ps(20_000_000_000)),
        Contact::Open
    );
    assert!(sw1.model.ratings().is_empty());

    let other_seed = json.replace(r#""seed": 7"#, r#""seed": 8"#);
    let reseeded = netlist::parse(other_seed.as_bytes()).unwrap();
    let Model::Switch(moved) = &reseeded.parts().next().unwrap().1.model else {
        panic!("not a switch");
    };
    assert_ne!(moved.schedule.edges(), schedule.edges());
    assert_eq!(moved.schedule.edges()[0], schedule.edges()[0]);
}

#[test]
fn the_next_edge_is_the_first_one_strictly_after_the_time() {
    let json = switch(
        r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 12000, "state": "open" }]"#,
        "null",
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let Model::Switch(switch) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a switch");
    };
    let next = |t| switch.schedule.next_change_after(Time::from_ps(t));
    assert_eq!(next(0), Some(Time::from_ps(5_000)));
    assert_eq!(next(4_999), Some(Time::from_ps(5_000)));
    assert_eq!(next(5_000), Some(Time::from_ps(12_000)));
    assert_eq!(next(5_001), Some(Time::from_ps(12_000)));
    assert_eq!(next(11_999), Some(Time::from_ps(12_000)));
    assert_eq!(next(12_000), None);
    assert_eq!(next(12_001), None);
    assert_eq!(next(u64::MAX), None);
}

/// `max_bounces` 1 and a 4 ns window leave no slack, so the two flips are 2 ns and 4 ns
/// after the event. Those times are the schedule, not the one event.
#[test]
fn the_next_edge_of_a_bounce_is_the_expanded_flip() {
    let json = switch(
        r#"[{ "t_ps": 10000, "state": "closed" }]"#,
        &bounce("4000", "1"),
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let Model::Switch(switch) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a switch");
    };
    let edges: Vec<u64> = switch
        .schedule
        .edges()
        .iter()
        .map(|edge| edge.at.as_ps())
        .collect();
    assert_eq!(edges, [10_000, 12_000, 14_000]);
    let next = |t| {
        switch
            .schedule
            .next_change_after(Time::from_ps(t))
            .map(|t| t.as_ps())
    };
    assert_eq!(next(0), Some(10_000));
    assert_eq!(next(10_000), Some(12_000));
    assert_eq!(next(12_000), Some(14_000));
    assert_eq!(next(13_999), Some(14_000));
    assert_eq!(next(14_000), None);
}

#[test]
fn each_broken_event_names_its_index_and_rule() {
    let cases = [
        (
            r#"[{ "t_ps": 0, "state": "closed" }]"#,
            "null",
            "at_zero",
            0,
        ),
        (
            r#"[{ "t_ps": 5, "state": "open" }]"#,
            "null",
            "same_state",
            0,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 6999, "state": "open" }]"#,
            "null",
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 9000, "state": "closed" }, { "t_ps": 5000, "state": "open" }]"#,
            "null",
            "not_increasing",
            1,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 5000, "state": "open" }]"#,
            "null",
            "not_increasing",
            1,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 10999, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 18446744073709550000, "state": "closed" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "overflow",
            0,
        ),
        (
            r#"[{ "t_ps": 18446744073709550000, "state": "closed" }, { "t_ps": 18446744073709551000, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "overflow",
            0,
        ),
        (
            r#"[{ "t_ps": 18446744073709550115, "state": "closed" }, { "t_ps": 18446744073709551115, "state": "open" }]"#,
            "null",
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 18446744073709545616, "state": "closed" }, { "t_ps": 18446744073709545716, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 10000, "state": "closed" }, { "t_ps": 10001, "state": "open" }]"#,
            r#"{ "window": { "value": 18446744073709540615, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 8, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
    ];
    for (events, bounce, rule, event) in cases {
        assert_eq!(
            errors(&switch(events, bounce)),
            [format!(
                r##""netlist.schedule" ["#sw1"] {{"event":{event},"rule":"{rule}"}}"##
            )],
            "{events}"
        );
    }
    let spaced = r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 7000, "state": "open" }]"#;
    assert!(netlist::parse(switch(spaced, "null").as_bytes()).is_ok());
    let after_window =
        r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 11000, "state": "open" }]"#;
    assert!(netlist::parse(switch(after_window, &bounce("4000", "1")).as_bytes()).is_ok());
    let gap_fits_at_the_top = r#"[{ "t_ps": 18446744073709549615, "state": "closed" }, { "t_ps": 18446744073709551615, "state": "open" }]"#;
    assert!(netlist::parse(switch(gap_fits_at_the_top, "null").as_bytes()).is_ok());
}

#[test]
fn bounce_values_follow_the_value_rules() {
    let events = r#"[{ "t_ps": 5000, "state": "closed" }]"#;
    assert_eq!(
        errors(&switch(events, &bounce("0", "1"))),
        [r##""netlist.value" ["#sw1"] {"field":"bounce.window","rule":"positive","value":0.0}"##]
    );
    assert_eq!(
        errors(&switch(events, &bounce("4000", "0"))),
        [
            r##""netlist.value" ["#sw1"] {"field":"bounce.max_bounces","rule":"positive","value":0.0}"##
        ]
    );
    assert_eq!(
        errors(&switch(events, &bounce("7999", "2"))),
        [
            r##""netlist.value" ["#sw1"] {"field":"bounce.window","rule":"bounce_room","value":7999.0}"##
        ]
    );
    assert!(netlist::parse(switch(events, &bounce("8000", "2")).as_bytes()).is_ok());
    let unit = switch(events, &bounce("4000", "1")).replace(r#""unit": "ps""#, r#""unit": "s""#);
    assert_eq!(
        errors(&unit),
        [r##""netlist.unit" ["#sw1"] {"expected":"ps","field":"bounce.window","found":"s"}"##]
    );
    for (window, max_bounces) in [
        ("4000.0", "1"),
        ("1e9", "1"),
        ("4000", "256"),
        ("4000", "-1"),
    ] {
        let err =
            netlist::parse(switch(events, &bounce(window, max_bounces)).as_bytes()).unwrap_err();
        assert_eq!(err[0].code(), "netlist.syntax", "{window} {max_bounces}");
    }
    let missing = switch(events, "null").replace(",\n          \"bounce\": null", "");
    assert!(!missing.contains("bounce"), "{missing}");
    let err = netlist::parse(missing.as_bytes()).unwrap_err();
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains("missing field `bounce`"), "{text}");
}

fn only_switch(netlist: &netlist::Netlist) -> &netlist::Switch {
    let Model::Switch(switch) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a switch");
    };
    switch
}

/// A switch with no events takes one at 1 ps. After an event the next waits out its bounce
/// window and the gap, and one picosecond sooner is `too_close`.
#[test]
fn the_earliest_next_event_is_the_first_one_parse_accepts() {
    let none = netlist::parse(switch("[]", &bounce("1000000", "4")).as_bytes()).unwrap();
    let earliest = |netlist: &netlist::Netlist, from| {
        only_switch(netlist)
            .earliest(Time::from_ps(from))
            .map(Time::as_ps)
    };
    assert_eq!(earliest(&none, 0), Some(1));
    assert_eq!(earliest(&none, 5), Some(5));
    assert_eq!(
        earliest(&none, u64::MAX - 1_000_000),
        Some(u64::MAX - 1_000_000)
    );
    assert_eq!(earliest(&none, u64::MAX - 999_999), None);

    let one = r#"{ "t_ps": 2000000000, "state": "closed" }"#;
    let bounced = switch(&format!("[{one}]"), &bounce("1000000", "4"));
    let first = 2_000_000_000 + 1_000_000 + 2_000;
    assert_eq!(
        earliest(&netlist::parse(bounced.as_bytes()).unwrap(), 0),
        Some(first)
    );
    let at = |t_ps: u64| {
        switch(
            &format!(r#"[{one}, {{ "t_ps": {t_ps}, "state": "open" }}]"#),
            &bounce("1000000", "4"),
        )
    };
    assert!(netlist::parse(at(first).as_bytes()).is_ok());
    assert_eq!(
        errors(&at(first - 1)),
        [r##""netlist.schedule" ["#sw1"] {"event":1,"rule":"too_close"}"##]
    );
}

/// Each appended event draws the bounce parse draws for an event at its index, so the
/// netlist equals the parse of a document that lists every event.
#[test]
fn an_appended_event_bounces_as_parse_bounces_it() {
    let event = |t_ps: u64, state: &str| format!(r#"{{ "t_ps": {t_ps}, "state": "{state}" }}"#);
    let document =
        |events: &[String]| switch(&format!("[{}]", events.join(", ")), &bounce("1000000", "4"));
    let mut appended =
        netlist::parse(document(&[event(2_000_000_000, "closed")]).as_bytes()).unwrap();
    let mut events = vec![event(2_000_000_000, "closed")];
    for (k, to) in [Contact::Open, Contact::Closed, Contact::Open]
        .into_iter()
        .enumerate()
    {
        let from = Time::from_ps(5_000_000_000 * (k as u64 + 1));
        let at = only_switch(&appended).earliest(from).unwrap();
        let id = appended.parts().next().unwrap().0;
        appended.append(id, netlist::Edge { at, to });
        let state = if to == Contact::Open {
            "open"
        } else {
            "closed"
        };
        events.push(event(at.as_ps(), state));
        let parsed = netlist::parse(document(&events).as_bytes()).unwrap();
        assert!(only_switch(&parsed).schedule.edges().len() > 2 * events.len());
        assert_eq!(
            only_switch(&appended),
            only_switch(&parsed),
            "event {}",
            k + 1
        );
    }
}
