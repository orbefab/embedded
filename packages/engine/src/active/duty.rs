//! The host's PWM input pin. It gives [`DutyTarget::duty`] each `Duty` record routed to the
//! pin.

use orbe_model::{Drive, DutyTarget, Time};

use super::native::{Hosting, Port};
use crate::link::{LinkRow, RecordKind};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pin;

impl<D: DutyTarget> Port<D> for Pin {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [Drive]) {}

    fn due_at(&self) -> Option<Time> {
        None
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _at: Time) -> bool {
        unreachable!("a PWM pin has nothing due")
    }

    fn input(&mut self, _dev: &mut Hosting<D>, _at: Time, _levels: &[bool]) {}

    fn take_records(&mut self, dev: &mut Hosting<D>, at: Time, records: &[LinkRow]) {
        for row in records {
            if let RecordKind::Duty { duty, .. } = &row.record.kind {
                dev.call(at, |device, cx| device.duty(cx, at, *duty));
            }
        }
    }
}
