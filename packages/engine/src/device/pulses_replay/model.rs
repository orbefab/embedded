//! A replay of pulse trains as a device. It holds `OUT` at its idle level for the whole run. The
//! host's replay engine sends its trains as `Pulses` records, each at its end.

use orbe_model::{Cx, Device, Drive, Drives, Error, Params, State, Stop, Time};

/// It drives `OUT`.
pub(crate) const OUTPUTS: usize = 1;

#[derive(Clone, Copy, Debug)]
pub struct PulsesReplay {
    idle: bool,
}

impl State for PulsesReplay {}

impl Device for PulsesReplay {
    fn new(params: &Params<'_>) -> Result<PulsesReplay, Error> {
        let idle = params.bool("idle").ok_or(Error("idle is a bool"))?;
        Ok(PulsesReplay { idle })
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::new([if self.idle { Drive::High } else { Drive::Low }])
    }
}
