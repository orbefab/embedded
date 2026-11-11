//! A device on the `orbe-model` contract, as a core. [`Native`] runs a device linked into the
//! engine beside the host engine of the bus it declares.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use orbe_model::{Cx, Device, Drive, Drives, Event, Levels, Shows, Stop, Time};

use crate::diagnostic::ProtocolRule;
use crate::link::{LinkRow, Record};

/// A device's core, with the contract every `Core` keeps.
pub(crate) trait Hosted: Send + fmt::Debug {
    fn run_until(&mut self, limit: Time) -> Option<Stop>;
    fn drive(&self) -> &[Drive];
    fn sample(&mut self, at: Time, volts: &[f64]);
    fn input(&mut self, at: Time, levels: &[bool]);
    fn event(&mut self, at: Time, event: Event<'_>);
    /// One call per landing. The port walks each span on the concrete device.
    fn take(&mut self, at: Time, records: &[LinkRow]);
    /// The part of one record sent by `at`, where the run ends inside it.
    fn cut(&mut self, at: Time, record: &Record);
    /// What the device shows, as it declares.
    fn shows(&self) -> Shows;
    /// What the device shows now.
    fn view(&self) -> &[u8];
    /// Each time what the device shows changed, in time order.
    fn shown(&self) -> &[Shown];
    /// Each rule its port or its device saw broken, at the first time.
    fn broke(&self) -> Vec<(ProtocolRule, Time)>;
    fn boxed(&self) -> Box<dyn Hosted>;
    /// Becomes a copy of `source`, a core of the same device.
    fn copy_from(&mut self, source: &dyn Hosted);
    fn as_any(&self) -> &dyn Any;
}

/// What a device showed from `at` on.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    pub(crate) at: Time,
    pub(crate) bytes: Arc<[u8]>,
}

/// A device and what its calls left besides its state. Every call on the device goes through
/// [`call`](Self::call) or [`run_until`](Self::run_until), which read its `Cx`.
pub(crate) struct Hosting<D> {
    device: D,
    /// The first rule a call broke, and its time.
    broke: Option<(ProtocolRule, Time)>,
    /// Shared with every copy, so a copy copies a pointer.
    shown: Arc<Vec<Shown>>,
}

impl<D: Clone> Clone for Hosting<D> {
    fn clone(&self) -> Self {
        Hosting {
            device: self.device.clone(),
            broke: self.broke,
            shown: Arc::clone(&self.shown),
        }
    }

    /// Keeps the pointer when it is already the source's, as it is on most commits.
    fn clone_from(&mut self, source: &Self) {
        self.device.clone_from(&source.device);
        self.broke = source.broke;
        if !Arc::ptr_eq(&self.shown, &source.shown) {
            self.shown = Arc::clone(&source.shown);
        }
    }
}

impl<D: Device> Hosting<D> {
    fn new(device: D) -> Hosting<D> {
        Hosting {
            device,
            broke: None,
            shown: Arc::new(Vec::new()),
        }
    }

    /// `call` on the device, then what it left in its `Cx`, at `at`.
    #[inline]
    pub(crate) fn call<R>(&mut self, at: Time, call: impl FnOnce(&mut D, &mut Cx) -> R) -> R {
        let mut cx = Cx::default();
        let out = call(&mut self.device, &mut cx);
        self.read(at, cx);
        out
    }

    /// The device's next stop up to `limit`. What it left in its `Cx` is at that stop, or at
    /// `limit` with none.
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let mut cx = Cx::default();
        let stop = self.device.run_until(&mut cx, limit);
        self.read(stop.map_or(limit, |stop| stop.at), cx);
        stop
    }

    #[inline]
    fn read(&mut self, at: Time, cx: Cx) {
        if cx.broken().is_some() || cx.shown() {
            self.keep(at, cx);
        }
    }

    #[cold]
    fn keep(&mut self, at: Time, cx: Cx) {
        if let Some((name, when)) = cx.broken() {
            let rule = ProtocolRule::named(name).unwrap_or_else(|| {
                panic!("a device broke rule {name}, which the engine does not know")
            });
            self.broke.get_or_insert((rule, when));
        }
        if cx.shown() {
            let bytes = self.device.view();
            match D::SHOWS {
                Shows::Nothing => panic!("a device that shows nothing ran show"),
                Shows::Leds { .. } => {
                    assert!(
                        bytes.len().is_multiple_of(3),
                        "a chain shows three bytes per led"
                    );
                }
            }
            Arc::make_mut(&mut self.shown).push(Shown {
                at,
                bytes: Arc::from(bytes),
            });
        }
    }
}

/// The host engine of the bus a device declares. It drives that bus's pins, ahead of the
/// device's own outputs in the image, and turns the levels it watches into calls on the device.
pub(crate) trait Port<D>: Clone + Send + 'static {
    /// How many outputs it drives.
    const PINS: usize;
    fn drive(&self, image: &mut [Drive]);
    /// When a drive of its pins, or a call it makes on its own, is next due.
    fn due_at(&self) -> Option<Time>;
    /// Applies what is due at `at`: a drive, or a call on the device.
    fn take_due(&mut self, dev: &mut Hosting<D>, at: Time);
    /// The chip's watch levels from `at`. The port reads them before the device does.
    fn input(&mut self, dev: &mut Hosting<D>, at: Time, levels: &[bool]);
    /// Each rule the port saw broken, at the first time.
    fn broke(&self, _found: &mut Vec<(ProtocolRule, Time)>) {}
    /// The records that end at `at`. A port with no record pin does nothing.
    fn take_records(&mut self, _dev: &mut Hosting<D>, _at: Time, _records: &[LinkRow]) {}
    /// The part of `record` sent by `at`. A port with no record pin does nothing.
    fn cut_record(&mut self, _dev: &mut Hosting<D>, _at: Time, _record: &Record) {}
}

/// The most outputs a native device's image holds, its port's and its own.
const IMAGE: usize = Drives::MAX + 8;

/// A device linked into the engine, and its port. A copy is a checkpoint.
#[derive(Clone)]
pub(crate) struct Native<D, P> {
    dev: Hosting<D>,
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
            dev: Hosting::new(device),
            port,
            drives: device.drive(),
            image: [Drive::Off; IMAGE],
            len: P::PINS + outputs,
        };
        native.refresh();
        native
    }

    #[cfg(test)]
    pub(crate) fn device(&self) -> &D {
        &self.dev.device
    }

    fn refresh(&mut self) {
        self.drives = self.dev.device.drive();
        self.port.drive(&mut self.image[..P::PINS]);
        for (k, drive) in self.image[P::PINS..self.len].iter_mut().enumerate() {
            *drive = self.drives.get(k);
        }
    }

    fn held(&self) -> bool {
        self.dev.device.drive() == self.drives
    }

    /// What the port has due at `at`. A call it makes there leaves the device's drives alone.
    fn take_due(&mut self, at: Time) {
        let drives = self.dev.device.drive();
        self.port.take_due(&mut self.dev, at);
        assert!(
            self.dev.device.drive() == drives,
            "a call the port makes never changes the drives"
        );
    }
}

impl<D: Device, P: Port<D>> Hosted for Native<D, P> {
    /// The device's next stop and its port's next due time, whichever comes first. One that
    /// falls at the same time as the other is the same stop.
    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let due = self.port.due_at().filter(|&due| due <= limit);
        let stop = self.dev.run_until(due.unwrap_or(limit));
        let stop = match (stop, due) {
            (Some(stop), Some(due)) if stop.at == due => {
                self.take_due(due);
                stop
            }
            (Some(stop), _) => stop,
            (None, Some(due)) => {
                self.take_due(due);
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
        self.dev.call(at, |device, cx| device.sample(cx, at, volts));
        assert!(self.held(), "a sample never changes the drives");
    }

    fn input(&mut self, at: Time, levels: &[bool]) {
        debug_assert!(
            self.port.due_at().is_none_or(|due| due > at),
            "a landing skipped a drive"
        );
        self.port.input(&mut self.dev, at, levels);
        self.dev.call(at, |device, cx| {
            device.input(cx, at, Levels::from_slice(levels));
        });
        assert!(self.held(), "an input never changes the drives");
    }

    fn event(&mut self, at: Time, event: Event<'_>) {
        self.dev.call(at, |device, cx| device.event(cx, at, event));
        assert!(self.held(), "an event never changes the drives");
    }

    fn take(&mut self, at: Time, records: &[LinkRow]) {
        self.port.take_records(&mut self.dev, at, records);
        assert!(self.held(), "a record never changes the drives");
    }

    fn cut(&mut self, at: Time, record: &Record) {
        self.port.cut_record(&mut self.dev, at, record);
        assert!(self.held(), "a record never changes the drives");
    }

    fn shows(&self) -> Shows {
        D::SHOWS
    }

    fn view(&self) -> &[u8] {
        self.dev.device.view()
    }

    fn shown(&self) -> &[Shown] {
        &self.dev.shown
    }

    fn broke(&self) -> Vec<(ProtocolRule, Time)> {
        let mut found = Vec::new();
        self.port.broke(&mut found);
        found.extend(self.dev.broke);
        found
    }

    fn boxed(&self) -> Box<dyn Hosted> {
        Box::new(self.clone())
    }

    fn copy_from(&mut self, source: &dyn Hosted) {
        let source = source
            .as_any()
            .downcast_ref::<Native<D, P>>()
            .expect("a core copies only a core of its own device");
        self.dev.clone_from(&source.dev);
        self.port.clone_from(&source.port);
        self.drives = source.drives;
        self.image = source.image;
        self.len = source.len;
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
