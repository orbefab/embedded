//! A device on the `orbe-model` contract, as a core. [`Native`] runs a device linked into the
//! engine beside the host engines of the buses it declares.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use orbe_model::{
    Cx, Device, Draw, Drawn, Drive, Drives, Event, Levels, Pulses, Quantity, Shows, Stop, Time,
    View,
};

use crate::diagnostic::ProtocolRule;
use crate::link::{Answers, LinkRow, Record};
use crate::world::World;

/// A device's core, with the contract every `Core` keeps. Each call that reaches the device
/// takes the world, and the device reads the quantities it declares in it at the call's time.
pub(crate) trait Hosted: Send + fmt::Debug {
    fn run_until(&mut self, world: &World, limit: Time) -> Option<Stop>;
    fn drive(&self) -> &[Drive];
    /// The device's draw at its last stop, which the stamp reads.
    fn draw(&self) -> Draw;
    fn sample(&mut self, world: &World, at: Time, volts: &[f64]);
    fn input(&mut self, world: &World, at: Time, levels: &[bool]);
    fn event(&mut self, world: &World, at: Time, event: Event<'_>);
    /// One call per landing. The port walks each record on the concrete device, and drives
    /// each byte the device answers into `answers`.
    fn take(&mut self, world: &World, at: Time, records: &[LinkRow], answers: &mut Answers);
    /// The part of one record sent by `at`, where the run ends inside it.
    fn cut(&mut self, world: &World, at: Time, record: &Record);
    /// The world quantities the device reads.
    fn reads(&self) -> &'static [Quantity];
    /// The records its port sent that it still holds, in the order it sent them.
    fn records(&self) -> &[Record];
    /// Hands over and drops each record it holds that ended at or before `through`.
    fn drain_records(&mut self, through: Time, sent: &mut dyn FnMut(Record));
    /// The record its port started by `at` and has not ended, when its stops up to `at` are done.
    fn in_flight(&self, at: Time) -> Option<Record>;
    /// Hands over and drops each pixel write it closed, with its time, in the order it closed
    /// them.
    fn drain_rows(&mut self, sent: &mut dyn FnMut(Time, Drawn));
    /// What the device shows, as it declares.
    fn shows(&self) -> Shows;
    /// What the device shows now.
    fn view(&self) -> View<'_>;
    /// Each time what an LED chain shows changed, in time order. A core keeps every one only
    /// once [`keep_shown`](Self::keep_shown) asked it to, and otherwise the last alone.
    fn shown(&self) -> Vec<Shown>;
    /// When what an LED chain shows last changed. `None` before the first change.
    fn latched(&self) -> Option<Time>;
    /// Keeps every change of what an LED chain shows, for a run that writes them.
    fn keep_shown(&mut self);
    /// Each rule its port or its device saw broken, at the first time.
    fn broke(&self) -> Vec<(ProtocolRule, Time)>;
    /// The first record its port dropped because a lane sat on another net than its pin, and
    /// that record's sender, an index in the run's senders.
    fn miswired(&self) -> Option<(Time, u16)>;
    fn boxed(&self) -> Box<dyn Hosted>;
    /// Becomes a copy of `source`, a core of the same device.
    fn copy_from(&mut self, source: &dyn Hosted);
    fn as_any(&self) -> &dyn Any;
}

/// What an LED chain showed from `at` on.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    pub(crate) at: Time,
    pub(crate) bytes: Arc<[u8]>,
}

/// What an LED chain showed, newest first, as a list each copy shares. A copy copies a pointer,
/// and a change adds one link however many copies share the list, so a chain that latches `n`
/// times costs `n` links. A copy that rewinds keeps the links it had, and the newer links go
/// with the last copy that holds them.
#[derive(Clone, Debug, Default)]
struct History(Option<Arc<Link>>);

#[derive(Debug)]
struct Link {
    shown: Shown,
    before: History,
}

impl History {
    /// Adds `shown` after the rest, or in place of the rest when `keep` is false.
    fn push(&mut self, shown: Shown, keep: bool) {
        let before = if keep {
            std::mem::take(self)
        } else {
            History::default()
        };
        self.0 = Some(Arc::new(Link { shown, before }));
    }

    fn last(&self) -> Option<&Shown> {
        self.0.as_deref().map(|link| &link.shown)
    }

    /// Oldest first.
    fn to_vec(&self) -> Vec<Shown> {
        let mut all = Vec::new();
        let mut at = self.0.as_deref();
        while let Some(link) = at {
            all.push(link.shown.clone());
            at = link.before.0.as_deref();
        }
        all.reverse();
        all
    }

    fn same(&self, other: &History) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// Drops a long list one link at a time, so it never recurses once per link.
impl Drop for History {
    fn drop(&mut self) {
        let mut next = self.0.take();
        while let Some(link) = next {
            next = match Arc::try_unwrap(link) {
                Ok(mut link) => link.before.0.take(),
                Err(_) => None,
            };
        }
    }
}

/// A device and what its calls left besides its state. Every call on the device goes through
/// [`call`](Self::call) or [`run_until`](Self::run_until), which read its `Cx`.
pub(crate) struct Hosting<D> {
    device: D,
    /// The first time a call broke each rule, by [`ProtocolRule::index`].
    broke: [Option<Time>; ProtocolRule::ALL.len()],
    /// Shared with every copy, so a copy copies a pointer.
    shown: History,
    /// Whether `shown` keeps every change, or the last alone. A run that writes `leds.csv`
    /// keeps them all. A session that writes nothing needs only the last, and keeping every
    /// one would grow without bound in a long session.
    keep: bool,
    /// The pixel writes it closed that a commit has not staged. A call that closes one is at a
    /// landing that commits it, so a copy holds none.
    drawn: Vec<(Time, Drawn)>,
    /// The train a call queued that its port has not taken. The port takes it in the same
    /// call on the core, so a copy holds none.
    queued: Option<(Time, Pulses)>,
}

impl<D: Device> Clone for Hosting<D> {
    fn clone(&self) -> Self {
        Hosting {
            device: self.device.clone(),
            broke: self.broke,
            shown: self.shown.clone(),
            keep: self.keep,
            drawn: self.drawn.clone(),
            queued: self.queued.clone(),
        }
    }

    /// Copies the device with [`State::copy_from`](orbe_model::State::copy_from), and keeps the
    /// pointer when it is already the source's, as it is on most commits.
    fn clone_from(&mut self, source: &Self) {
        self.device.copy_from(&source.device);
        self.broke = source.broke;
        if !self.shown.same(&source.shown) {
            self.shown = source.shown.clone();
        }
        self.keep = source.keep;
        self.drawn.clone_from(&source.drawn);
        self.queued.clone_from(&source.queued);
    }
}

impl<D: Device> Hosting<D> {
    fn new(device: D) -> Hosting<D> {
        Hosting {
            device,
            broke: [None; ProtocolRule::ALL.len()],
            shown: History::default(),
            keep: false,
            drawn: Vec::new(),
            queued: None,
        }
    }

    /// A context in which the device reads what it declares of `world` at `at`.
    #[inline]
    fn cx(world: &World, at: Time) -> Cx {
        if D::WORLD.is_empty() {
            Cx::default()
        } else {
            Cx::new(world.readings(D::WORLD, at))
        }
    }

    /// `call` on the device, then what it left in its `Cx`, at `at`.
    #[inline]
    pub(crate) fn call<R>(
        &mut self,
        world: &World,
        at: Time,
        call: impl FnOnce(&mut D, &mut Cx) -> R,
    ) -> R {
        let mut cx = Self::cx(world, at);
        let out = call(&mut self.device, &mut cx);
        self.read(at, cx);
        out
    }

    /// [`call`](Self::call), in a `Cx` that says it replays a record.
    #[inline]
    pub(crate) fn replay<R>(
        &mut self,
        world: &World,
        at: Time,
        call: impl FnOnce(&mut D, &mut Cx) -> R,
    ) -> R {
        let mut cx = Self::cx(world, at).replaying();
        let out = call(&mut self.device, &mut cx);
        self.read(at, cx);
        out
    }

    /// The device's next stop up to `limit`. What it left in its `Cx` is at that stop, or at
    /// `limit` with none. It reads the world at `limit`: a change of a quantity it reads lands
    /// it, so no run crosses one.
    fn run_until(&mut self, world: &World, limit: Time) -> Option<Stop> {
        let mut cx = Self::cx(world, limit);
        let stop = self.device.run_until(&mut cx, limit);
        self.read(stop.map_or(limit, |stop| stop.at), cx);
        stop
    }

    /// The train a call queued, which only its port takes.
    pub(crate) fn take_queued(&mut self) -> Option<(Time, Pulses)> {
        self.queued.take()
    }

    #[inline]
    fn read(&mut self, at: Time, cx: Cx) {
        if cx.broken().is_some() || cx.shown() || cx.drawn().is_some() || cx.queued().is_some() {
            self.keep(at, cx);
        }
    }

    #[cold]
    fn keep(&mut self, at: Time, cx: Cx) {
        if let Some((name, when)) = cx.broken() {
            let rule = ProtocolRule::named(name).unwrap_or_else(|| {
                panic!("a device broke rule {name}, which the engine does not know")
            });
            self.broke[rule.index()].get_or_insert(when);
        }
        if cx.shown() {
            let View::Leds(bytes) = self.device.view() else {
                panic!("only an LED chain runs show");
            };
            assert!(
                matches!(D::SHOWS, Shows::Leds { .. }) && bytes.len().is_multiple_of(3),
                "a chain shows three bytes per led"
            );
            let shown = Shown {
                at,
                bytes: Arc::from(bytes),
            };
            self.shown.push(shown, self.keep);
        }
        if let Some(drawn) = cx.drawn() {
            assert!(
                matches!(D::SHOWS, Shows::Screen { .. }),
                "only a screen draws"
            );
            self.drawn.push((at, drawn));
        }
        if let Some((start, train)) = cx.queued() {
            assert!(*start > at, "a train starts after the call that queues it");
            assert!(
                self.queued.is_none(),
                "the port takes each train before the next call"
            );
            self.queued = Some((*start, train.clone()));
        }
    }
}

/// The host engine of the bus a device declares. It drives that bus's pins, ahead of the
/// device's own outputs in the image, and turns the levels it watches into calls on the device.
pub(crate) trait Port<D>: Clone + Send + 'static {
    /// How many outputs it drives.
    const PINS: usize;
    fn drive(&self, image: &mut [Drive]);
    /// When a drive of its pins, a call it makes on its own, or the end of a record it sends is
    /// next due.
    fn due_at(&self) -> Option<Time>;
    /// Applies what is due at `at`: a drive, a call on the device, or a record's end. Returns
    /// whether a record it sent ends there.
    fn take_due(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) -> bool;
    /// The chip's watch levels from `at`. The port reads them before the device does.
    fn input(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, levels: &[bool]);
    /// Takes the train a call on the device queued at `at`. Only a port that plays trains
    /// takes one.
    fn claim(&mut self, _dev: &mut Hosting<D>, _at: Time) {}
    /// Each rule the port saw broken, at the first time.
    fn broke(&self, _found: &mut Vec<(ProtocolRule, Time)>) {}
    /// The records that end at `at`, and the bytes the device answers on MISO to those that
    /// read. A port with no record pin does nothing.
    fn take_records(
        &mut self,
        _dev: &mut Hosting<D>,
        _world: &World,
        _at: Time,
        _records: &[LinkRow],
        _answers: &mut Answers,
    ) {
    }
    /// The part of `record` sent by `at`. A port with no record pin does nothing.
    fn cut_record(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time, _record: &Record) {}
    /// The records it sent that it still holds. A port that sends none holds none.
    fn records(&self) -> &[Record] {
        &[]
    }
    /// Hands over and drops each record it holds that ended at or before `through`.
    fn drain_records(&mut self, _through: Time, _sent: &mut dyn FnMut(Record)) {}
    /// The record it started by `at` and has not ended.
    fn in_flight(&self, _at: Time) -> Option<Record> {
        None
    }
    /// The first record it dropped because a lane sat on another net than its pin.
    fn miswired(&self) -> Option<(Time, u16)> {
        None
    }
}

/// Two host engines on one device, such as an SPI bus and a PWM pin. The first one's pins come
/// first in the image. A device takes the records of a landing bus by bus.
impl<D, A: Port<D>, B: Port<D>> Port<D> for (A, B) {
    const PINS: usize = A::PINS + B::PINS;

    fn drive(&self, image: &mut [Drive]) {
        let (a, b) = image.split_at_mut(A::PINS);
        self.0.drive(a);
        self.1.drive(b);
    }

    fn due_at(&self) -> Option<Time> {
        match (self.0.due_at(), self.1.due_at()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn take_due(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) -> bool {
        let a = self.0.due_at() == Some(at) && self.0.take_due(dev, world, at);
        let b = self.1.due_at() == Some(at) && self.1.take_due(dev, world, at);
        a || b
    }

    fn input(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, levels: &[bool]) {
        self.0.input(dev, world, at, levels);
        self.1.input(dev, world, at, levels);
    }

    fn claim(&mut self, dev: &mut Hosting<D>, at: Time) {
        self.0.claim(dev, at);
        self.1.claim(dev, at);
    }

    fn broke(&self, found: &mut Vec<(ProtocolRule, Time)>) {
        self.0.broke(found);
        self.1.broke(found);
    }

    fn take_records(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        at: Time,
        records: &[LinkRow],
        answers: &mut Answers,
    ) {
        self.0.take_records(dev, world, at, records, answers);
        self.1.take_records(dev, world, at, records, answers);
    }

    fn cut_record(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, record: &Record) {
        self.0.cut_record(dev, world, at, record);
        self.1.cut_record(dev, world, at, record);
    }

    fn records(&self) -> &[Record] {
        debug_assert!(self.1.records().is_empty(), "one bus of a device sends");
        self.0.records()
    }

    fn drain_records(&mut self, through: Time, sent: &mut dyn FnMut(Record)) {
        self.0.drain_records(through, sent);
    }

    fn in_flight(&self, at: Time) -> Option<Record> {
        self.0.in_flight(at)
    }

    fn miswired(&self) -> Option<(Time, u16)> {
        self.0.miswired().or(self.1.miswired())
    }
}

/// The most outputs a native device's image holds, its port's and its own.
const IMAGE: usize = Drives::MAX + 8;

/// A device linked into the engine, and its port. A copy is a checkpoint.
pub(crate) struct Native<D, P> {
    dev: Hosting<D>,
    port: P,
    /// The device's drives after its last stop.
    drives: Drives,
    /// The device's draw after its last stop.
    draw: Draw,
    /// A stop 1 ps after a call other than `run_until` changed the device's draw.
    redraw: Option<Time>,
    /// The port's drives, then the device's, up to `len`.
    image: [Drive; IMAGE],
    len: usize,
}

impl<D: Device, P: Clone> Clone for Native<D, P> {
    fn clone(&self) -> Self {
        Native {
            dev: self.dev.clone(),
            port: self.port.clone(),
            drives: self.drives,
            draw: self.draw,
            redraw: self.redraw,
            image: self.image,
            len: self.len,
        }
    }
}

impl<D: Device, P: Port<D>> Native<D, P> {
    /// `device` at power-on, which drives `outputs` outputs of its own.
    /// `refresh` fills the drives after the device has moved.
    pub(crate) fn new(device: D, port: P, outputs: usize) -> Native<D, P> {
        assert!(outputs <= Drives::MAX && P::PINS + outputs <= IMAGE);
        let mut native = Native {
            dev: Hosting::new(device),
            port,
            drives: Drives::OFF,
            draw: Draw::NONE,
            redraw: None,
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
        self.draw = self.dev.device.draw();
        self.redraw = None;
        self.port.drive(&mut self.image[..P::PINS]);
        for (k, drive) in self.image[P::PINS..self.len].iter_mut().enumerate() {
            *drive = self.drives.get(k);
        }
    }

    fn held(&self) -> bool {
        self.dev.device.drive() == self.drives
    }

    /// After a call at `at` other than `run_until`: a draw that call changed takes effect at a
    /// stop 1 ps later, or at an earlier stop already due.
    fn follow_draw(&mut self, at: Time) {
        if self.dev.device.draw() != self.draw && self.redraw.is_none() {
            self.redraw = Some(Time::from_ps(at.as_ps() + 1));
        }
    }

    /// Hands the port the train a call at `at` queued. A device on a port that plays no train
    /// queues none.
    fn claim(&mut self, at: Time) {
        self.port.claim(&mut self.dev, at);
        assert!(
            self.dev.queued.is_none(),
            "only a pulse target on an open-drain line queues a train"
        );
    }

    /// What the port has due at `at`, and whether a record it sent ends there. A call it makes
    /// there leaves the device's drives alone.
    fn take_due(&mut self, world: &World, at: Time) -> bool {
        let drives = self.dev.device.drive();
        let record = self.port.take_due(&mut self.dev, world, at);
        assert!(
            self.dev.device.drive() == drives,
            "a call the port makes never changes the drives"
        );
        record
    }
}

impl<D: Device, P: Port<D>> Hosted for Native<D, P> {
    /// The device's next stop and its port's next due time, whichever comes first. One that
    /// falls at the same time as the other is the same stop.
    fn run_until(&mut self, world: &World, limit: Time) -> Option<Stop> {
        let due = self.port.due_at().filter(|&due| due <= limit);
        let redraw = self.redraw.filter(|&redraw| redraw <= limit);
        let bound = due.into_iter().chain(redraw).min();
        let stop = self.dev.run_until(world, bound.unwrap_or(limit));
        self.claim(stop.map_or(bound.unwrap_or(limit), |stop| stop.at));
        let at = stop.map_or(bound, |stop| Some(stop.at));
        let at = at?;
        let record = due == Some(at) && self.take_due(world, at);
        let stop = Stop {
            at,
            sample: stop.is_some_and(|stop| stop.sample),
            record: stop.is_some_and(|stop| stop.record) || record,
        };
        self.refresh();
        Some(stop)
    }

    fn drive(&self) -> &[Drive] {
        &self.image[..self.len]
    }

    fn draw(&self) -> Draw {
        self.draw
    }

    fn sample(&mut self, world: &World, at: Time, volts: &[f64]) {
        self.dev
            .call(world, at, |device, cx| device.sample(cx, at, volts));
        self.claim(at);
        assert!(self.held(), "a sample never changes the drives");
        self.follow_draw(at);
    }

    fn input(&mut self, world: &World, at: Time, levels: &[bool]) {
        debug_assert!(
            self.port.due_at().is_none_or(|due| due > at),
            "a landing skipped a drive"
        );
        self.port.input(&mut self.dev, world, at, levels);
        self.dev.call(world, at, |device, cx| {
            device.input(cx, at, Levels::from_slice(levels));
        });
        self.claim(at);
        assert!(self.held(), "an input never changes the drives");
        self.follow_draw(at);
    }

    fn event(&mut self, world: &World, at: Time, event: Event<'_>) {
        self.dev
            .call(world, at, |device, cx| device.event(cx, at, event));
        self.claim(at);
        assert!(self.held(), "an event never changes the drives");
        self.follow_draw(at);
    }

    fn take(&mut self, world: &World, at: Time, records: &[LinkRow], answers: &mut Answers) {
        self.port
            .take_records(&mut self.dev, world, at, records, answers);
        self.claim(at);
        assert!(self.held(), "a record never changes the drives");
        self.follow_draw(at);
    }

    fn cut(&mut self, world: &World, at: Time, record: &Record) {
        self.port.cut_record(&mut self.dev, world, at, record);
        self.claim(at);
        assert!(self.held(), "a record never changes the drives");
        self.follow_draw(at);
    }

    fn reads(&self) -> &'static [Quantity] {
        D::WORLD
    }

    fn records(&self) -> &[Record] {
        self.port.records()
    }

    fn drain_records(&mut self, through: Time, sent: &mut dyn FnMut(Record)) {
        self.port.drain_records(through, sent);
    }

    fn in_flight(&self, at: Time) -> Option<Record> {
        self.port.in_flight(at)
    }

    fn drain_rows(&mut self, sent: &mut dyn FnMut(Time, Drawn)) {
        for (at, drawn) in self.dev.drawn.drain(..) {
            sent(at, drawn);
        }
    }

    fn shows(&self) -> Shows {
        D::SHOWS
    }

    fn view(&self) -> View<'_> {
        self.dev.device.view()
    }

    fn shown(&self) -> Vec<Shown> {
        self.dev.shown.to_vec()
    }

    fn latched(&self) -> Option<Time> {
        self.dev.shown.last().map(|shown| shown.at)
    }

    fn keep_shown(&mut self) {
        self.dev.keep = true;
    }

    fn broke(&self) -> Vec<(ProtocolRule, Time)> {
        let mut found = Vec::new();
        self.port.broke(&mut found);
        found.extend(
            ProtocolRule::ALL
                .into_iter()
                .zip(self.dev.broke)
                .filter_map(|(rule, at)| at.map(|at| (rule, at))),
        );
        found
    }

    fn miswired(&self) -> Option<(Time, u16)> {
        self.port.miswired()
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
        self.draw = source.draw;
        self.redraw = source.redraw;
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
