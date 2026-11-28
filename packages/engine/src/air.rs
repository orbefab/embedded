//! The air between radio stations, which the engine owns. The stations are each active core
//! with a radio, in slot order, then the network. A frame lands at its end, and the air gives
//! it to every other station tuned to its channel there, ORs their answers and hands the `Ack`
//! to its sender before the sender runs on. `docs/engine/engine.md`, The air, holds the rules.

use orbe_model::air::{Ack, AirFrame, Channel, Signal};
use orbe_model::{Stop, Time};
use orbe_net::{Network, Row};

use crate::active::{Chip, Core};

/// How every station hears every other. The air models no distance and no fading.
pub(crate) const SIGNAL: Signal = Signal {
    rssi_dbm: -50,
    noise_dbm: -95,
};

/// The network as an active core with no part: no pins, no drives, and its frames on the air.
/// `Clone` is the checkpoint, and its scene data sits behind an `Arc` every copy shares.
#[derive(Clone, Debug)]
pub(crate) struct Net {
    network: Network,
    /// The frame that ended at its last stop, until the landing there hands it on.
    aired: Option<AirFrame>,
    /// The rows it made, in order, until a delivery stages them.
    rows: Vec<Row>,
}

impl Net {
    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let stop = self.network.run_until(limit);
        if stop.is_some_and(|stop| stop.record) {
            debug_assert!(self.aired.is_none(), "a landing hands on each frame");
            self.network.take_aired(|frame| self.aired = Some(frame));
        }
        self.take_rows();
        stop
    }

    pub(crate) fn tuned(&self) -> Option<Channel> {
        self.network.tuned()
    }

    pub(crate) fn aired(&self) -> Option<&AirFrame> {
        self.aired.as_ref()
    }

    pub(crate) fn drop_aired(&mut self) {
        self.aired = None;
    }

    pub(crate) fn hear(&mut self, at: Time, frame: &AirFrame, signal: Signal) -> Ack {
        let ack = self.network.hear(at, frame, signal);
        self.take_rows();
        ack
    }

    pub(crate) fn acked(&mut self, at: Time, ack: Ack) {
        self.network.acked(at, ack);
        self.take_rows();
    }

    #[cfg(test)]
    pub(crate) fn reached(&self) -> Time {
        self.network.reached()
    }

    #[cfg(test)]
    pub(crate) fn send_msdu(&mut self, at: Time, sta: orbe_net::Mac, msdu: &[u8]) {
        self.network.send_msdu(at, sta, msdu);
    }

    /// Hands over and drops each row it made at or before `through`, in order.
    pub(crate) fn drain_rows(&mut self, through: Time, made: impl FnMut(Row)) {
        let ended = self.rows.partition_point(|row| row.at <= through);
        self.rows.drain(..ended).for_each(made);
    }

    fn take_rows(&mut self) {
        let rows = &mut self.rows;
        self.network.take_rows(|row| {
            debug_assert!(rows.last().is_none_or(|last| last.at <= row.at));
            rows.push(row);
        });
    }
}

/// The active chip of the netlist's `network`.
pub(crate) fn chip(config: &orbe_net::Config) -> Chip {
    Chip {
        part: None,
        outputs: Vec::new(),
        watches: Vec::new(),
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        draw: None,
        takes: Default::default(),
        core: Core::Net(Box::new(Net {
            network: Network::new(config.clone()),
            aired: None,
            rows: Vec::new(),
        })),
    }
}

/// The stations, by slot: each core with a radio in slot order, then the network.
pub(crate) struct Air {
    stations: Vec<usize>,
    /// Each frame a landing handed on, with its sender's slot, in landing order. A landing run
    /// again after a rewind logs its frames again.
    #[cfg(test)]
    pub(crate) log: Vec<(usize, AirFrame)>,
    /// The frames whose sender hears no ACK, though a station took them, as if the ACK were
    /// lost on the air. It reads only the frame, so a landing run again loses the same ones.
    #[cfg(test)]
    pub(crate) lose_ack: Option<fn(&AirFrame) -> bool>,
}

impl Air {
    /// `None` without the network, as in a netlist without `network`.
    pub(crate) fn new(cores: &[Core]) -> Option<Air> {
        let net = |core: &Core| matches!(core, Core::Net(_));
        if !cores.iter().any(net) {
            return None;
        }
        let slots = || cores.iter().enumerate();
        let radios = slots().filter(|(_, core)| core.radio() && !net(core));
        let nets = slots().filter(|(_, core)| net(core));
        let stations = radios.chain(nets).map(|(slot, _)| slot).collect();
        Some(Air {
            stations,
            #[cfg(test)]
            log: Vec::new(),
            #[cfg(test)]
            lose_ack: None,
        })
    }

    /// The slot of each station, in station order.
    pub(crate) fn stations(&self) -> &[usize] {
        &self.stations
    }

    pub(crate) fn is_station(&self, slot: usize) -> bool {
        self.stations.contains(&slot)
    }
}
