//! A network is its state. A run split at any limits is the same run, and a copy taken
//! anywhere in an exchange runs on as the network it copied would have.

mod common;

use common::{Air, Rng, Seen, keyed_network, ms, session};
use orbe_model::Time;
use orbe_net::Network;

/// The session run in one go, with no limit but the script's own times.
fn whole() -> (Air, Network) {
    let (script, until) = session();
    let mut air = Air::new(script);
    let mut net = keyed_network();
    air.run_through(&mut net, until);
    (air, net)
}

/// A limit in `(from, to)`, mostly close to `from` and sometimes anywhere, or none.
fn random_limit(rng: &mut Rng, from: Time, to: Time) -> Option<Time> {
    let span = to.as_ps() - from.as_ps();
    if span < 2 || rng.below(8) == 0 {
        return None;
    }
    let reach = 1u64 << rng.below(40);
    Some(Time::from_ps(
        from.as_ps() + 1 + rng.below(reach.min(span - 1)),
    ))
}

#[test]
fn the_session_reaches_every_part_of_the_access_point() {
    let (air, mut net) = whole();
    let rows = air.rows_seen().join("\n");
    for event in [
        "auth",
        "assoc",
        "handshake",
        "addba_declined",
        "doze",
        "wake",
        "disassoc",
        "ap_down",
        "ap_up",
        "deauth",
    ] {
        assert!(
            rows.contains(&format!(r#""event":"{event}""#)),
            "{event} in\n{rows}"
        );
    }
    assert!(air.aired().len() > 10);
    // The station's Data frame, once: the second is a replay.
    let mut delivered = 0;
    net.take_delivered(|_, _| delivered += 1);
    assert_eq!(delivered, 1);
}

#[test]
fn a_run_split_at_random_limits_is_the_same_run() {
    let (want, want_net) = whole();
    for seed in 1..=64u64 {
        let (script, until) = session();
        let mut air = Air::new(script);
        let mut net = keyed_network();
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        air.run(&mut net, until, &mut |from, to| {
            random_limit(&mut rng, from, to)
        });
        assert_eq!(air.seen, want.seen, "seed {seed}");
        assert_eq!(net, want_net, "seed {seed}");
    }
}

/// Every time a frame starts or ends, or the station's next frame lands, and a picosecond
/// either side: the places a copy is most likely to drop something.
fn cuts(air: &Air) -> Vec<Time> {
    let mut cuts = Vec::new();
    for seen in &air.seen {
        let times = match seen {
            Seen::Aired(frame, _) => vec![frame.start, frame.end],
            Seen::Heard(at, _) => vec![*at],
            Seen::Row(_) => vec![],
        };
        for t in times {
            for ps in [t.as_ps().saturating_sub(1), t.as_ps(), t.as_ps() + 1] {
                cuts.push(Time::from_ps(ps));
            }
        }
    }
    cuts.push(ms(55.0));
    cuts
}

#[test]
fn a_copy_taken_mid_exchange_runs_on_as_the_network_would() {
    let (want, want_net) = whole();
    let (script, until) = session();
    for cut in cuts(&want) {
        let mut air = Air::new(script.clone());
        let mut net = keyed_network();
        air.run_through(&mut net, cut);

        // A copy, run on in place of the network.
        let mut copy_air = air.clone();
        let mut copy = net.clone();
        copy_air.run_through(&mut copy, until);
        assert_eq!(copy_air.seen, want.seen, "copy at {cut:?}");
        assert_eq!(copy, want_net, "copy at {cut:?}");

        // A copy run ahead, then rewound onto the network with `clone_from`, as the engine
        // rewinds a core onto its last commit.
        let mut ahead_air = air.clone();
        let mut ahead = net.clone();
        ahead_air.run_through(&mut ahead, Time::from_ps(cut.as_ps() + 30_000_000_000));
        ahead.clone_from(&net);
        air.run_through(&mut ahead, until);
        assert_eq!(air.seen, want.seen, "rewound at {cut:?}");
        assert_eq!(ahead, want_net, "rewound at {cut:?}");
    }
}
