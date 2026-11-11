//! A device on the `orbe-model` contract, as a core. [`Native`] runs a device linked into the
//! engine beside the host engine of the bus it declares.

use std::any::Any;
use std::fmt;

use orbe_model::{Cx, Device, Drive, Drives, Event, Levels, Stop, Time};

use crate::diagnostic::ProtocolRule;

/// A device's core, with the contract every `Core` keeps.
pub(crate) trait Hosted: Send + fmt::Debug {
    fn run_until(&mut self, limit: Time) -> Option<Stop>;
    fn drive(&self) -> &[Drive];
    fn sample(&mut self, at: Time, volts: &[f64]);
    fn input(&mut self, at: Time, levels: &[bool]);
    fn event(&mut self, at: Time, event: Event<'_>);
    /// Each rule its port saw broken, at the first time.
    fn broke(&self) -> Vec<(ProtocolRule, Time)>;
    fn boxed(&self) -> Box<dyn Hosted>;
    /// Becomes a copy of `source`, a core of the same device.
    fn copy_from(&mut self, source: &dyn Hosted);
    fn as_any(&self) -> &dyn Any;
}

/// The host engine of the bus a device declares. It drives that bus's pins, ahead of the
/// device's own outputs in the image, and turns the levels it watches into calls on the device.
pub(crate) trait Port<D>: Copy + Send + 'static {
    /// How many outputs it drives.
    const PINS: usize;
    fn drive(&self, image: &mut [Drive]);
    /// When a drive of its pins is next due.
    fn due_at(&self) -> Option<Time>;
    /// Applies the drive due at `at`.
    fn take_due(&mut self, at: Time);
    /// The chip's watch levels from `at`. The port reads them before the device does.
    fn input(&mut self, device: &mut D, cx: &mut Cx, at: Time, levels: &[bool]);
    fn broke(&self, found: &mut Vec<(ProtocolRule, Time)>);
}

/// The most outputs a native device's image holds, its port's and its own.
const IMAGE: usize = Drives::MAX + 8;

/// A device linked into the engine, and its port. A copy is a checkpoint.
#[derive(Clone, Copy)]
pub(crate) struct Native<D, P> {
    device: D,
    port: P,
    /// The device's drives after its last stop.
    drives: Drives,
    /// The port's drives, then the device's, up to `len`.
    image: [Drive; IMAGE],
    len: usize,
}

impl<D: Device, P: Port<D>> Native<D, P> {
    /// `device` at power-on, which drives `outputs` outputs of its own.
    pub(crate) fn new(device: D, port: P, outputs: usize) -> Native<D, P> {
        assert!(outputs <= Drives::MAX && P::PINS + outputs <= IMAGE);
        let mut native = Native {
            device,
            port,
            drives: device.drive(),
            image: [Drive::Off; IMAGE],
            len: P::PINS + outputs,
        };
        native.refresh();
        native
    }

    fn refresh(&mut self) {
        self.drives = self.device.drive();
        self.port.drive(&mut self.image[..P::PINS]);
        for (k, drive) in self.image[P::PINS..self.len].iter_mut().enumerate() {
            *drive = self.drives.get(k);
        }
    }

    fn held(&self) -> bool {
        self.device.drive() == self.drives
    }
}

impl<D: Device, P: Port<D>> Hosted for Native<D, P> {
    /// The device's next stop and its port's next drive, whichever comes first. One that falls
    /// at the same time as the other is the same stop.
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let due = self.port.due_at().filter(|&due| due <= limit);
        let stop = self
            .device
            .run_until(&mut Cx::default(), due.unwrap_or(limit));
        let stop = match (stop, due) {
            (Some(stop), Some(due)) if stop.at == due => {
                self.port.take_due(due);
                stop
            }
            (Some(stop), _) => stop,
            (None, Some(due)) => {
                self.port.take_due(due);
                Stop {
                    at: due,
                    sample: false,
                    record: false,
                }
            }
            (None, None) => return None,
        };
        self.refresh();
        Some(stop)
    }

    fn drive(&self) -> &[Drive] {
        &self.image[..self.len]
    }

    fn sample(&mut self, at: Time, volts: &[f64]) {
        self.device.sample(&mut Cx::default(), at, volts);
        assert!(self.held(), "a sample never changes the drives");
    }

    fn input(&mut self, at: Time, levels: &[bool]) {
        debug_assert!(
            self.port.due_at().is_none_or(|due| due > at),
            "a landing skipped a drive"
        );
        let mut cx = Cx::default();
        self.port.input(&mut self.device, &mut cx, at, levels);
        self.device.input(&mut cx, at, Levels::from_slice(levels));
        assert!(self.held(), "an input never changes the drives");
    }

    fn event(&mut self, at: Time, event: Event<'_>) {
        self.device.event(&mut Cx::default(), at, event);
        assert!(self.held(), "an event never changes the drives");
    }

    fn broke(&self) -> Vec<(ProtocolRule, Time)> {
        let mut found = Vec::new();
        self.port.broke(&mut found);
        found
    }

    fn boxed(&self) -> Box<dyn Hosted> {
        Box::new(*self)
    }

    fn copy_from(&mut self, source: &dyn Hosted) {
        *self = *source
            .as_any()
            .downcast_ref::<Native<D, P>>()
            .expect("a core copies only a core of its own device");
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<D, P> fmt::Debug for Native<D, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Native")
            .field("device", &std::any::type_name::<D>())
            .field("image", &&self.image[..self.len])
            .finish_non_exhaustive()
    }
}
