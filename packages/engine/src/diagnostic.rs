use orbe_mcu::{FlashRefusal, ResetReason, Setting, Unsupported};
use orbe_model::Time;

use crate::netlist::{Average, Basis, MIN_GAP_PS};

use std::borrow::Cow;
use std::num::NonZeroU64;

use serde::{Serialize, Serializer};

use crate::blob::{Role, Sha256, Source};
use crate::name::{Name, NameError, Ref};

/// `Error` means the engine has no trustworthy result.
/// `Warning` means the result stands and the circuit still has a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// Which numeric rule a netlist value broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueRule {
    Positive,
    FiniteReciprocal,
    /// `n*VT` is not normal, or `vcrit` is not finite and positive.
    /// On a zener, `nbvt` is not normal, or `Isb` or its `vcrit` is not.
    FiniteJunction,
    /// `bounce.window` is less than `2 * max_bounces * MIN_GAP_PS`.
    BounceRoom,
    NonNegative,
    /// Not inside `(0, 1)`.
    Fraction,
    /// Not at least 0 and below 1. Zero is allowed. One is not.
    BelowOne,
    /// A threshold's `low` is above its `high`.
    AboveHigh,
    /// A relay's `drop_out_current` is at or above its `pull_in_current`.
    Hysteresis,
    /// A zener's `zener_impedance` does not exceed `series_resistance + breakdown_resistance`.
    AboveResistance,
    /// A zener's `zener_voltage` does not exceed `zener_current` times those two resistances.
    AboveDrop,
    /// A fuse byte selects a mode the model does not run.
    Unsupported,
    /// A device event's integer is outside its span, such as a touch coordinate outside 0 to
    /// 4095.
    Span,
    /// An LED count is outside 1 to 1024.
    Leds,
    /// A `ws2812`'s `dout.delay` is past 1 µs.
    Delay,
    /// A world quantity's value, or a B equation's reference temperature, is outside the range
    /// its quantity's row of the table gives.
    Range,
    /// A world-driven field's curve is not finite at an end of its quantity's range.
    Finite,
    /// A linear curve's two points share their world value.
    Distinct,
}

/// Which rule a world quantity's name broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldRule {
    /// The name is not a quantity of the table.
    Unknown,
    /// A part reads a quantity the netlist's `world` does not declare.
    Undeclared,
    /// A B-equation curve reads a quantity other than temperature.
    Temperature,
    /// A power curve reads a quantity whose range goes below 0, where `w / x` is negative.
    NonNegative,
}

/// Why `firmware` is not a program for the part's flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareRule {
    /// The line is not an Intel HEX record: no `:`, a digit that is not hexadecimal, or a
    /// length that does not match its byte count or its type.
    Record,
    /// The record's bytes do not sum to 0.
    Checksum,
    /// A record type other than 00 to 05.
    RecordType,
    /// A record after the end-of-file record.
    AfterEnd,
    /// No end-of-file record.
    NoEnd,
    /// No data record, so the flash is erased: an `.eep` of a sketch with no EEPROM data, for
    /// one, which holds only the end-of-file record.
    Empty,
    /// A data byte at or past the end of the flash.
    PastFlash,
    /// A data byte an earlier record already placed, or an ELF segment on a byte of memory
    /// an earlier segment reaches.
    Overlap,
    /// Not a 32-bit little-endian Xtensa ELF executable with a loadable segment, or a header,
    /// segment or symbol table that runs past the file.
    Elf,
    /// An ELF or app segment outside the memory a program may load into, or an entry outside
    /// the segments in code memory.
    Unmapped,
    /// A flash region past the end of the flash, or on a byte an earlier region placed.
    Region,
    /// A flash with no partition table, no app partition the bootloader would pick, a set
    /// otadata, or an app image that is not one for the chip or fails its checksum or hash.
    Image,
    /// A `rom` that is not the one ROM the model runs, `rom` absent with `firmware.flash`, or
    /// `rom` present with `firmware.elf`.
    Rom,
}

/// Why a blob a netlist names is not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobRule {
    /// No blob is filed under its hash.
    Missing,
    /// The blob filed under its hash hashes to another digest.
    Hash,
    /// The blob has another length than the netlist's `bytes`.
    Length,
}

/// Which rule a transient analysis broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisRule {
    Positive,
    /// `max_step_ps` is below 1000, the first step of a segment.
    #[serde(rename = "step_under_1ns")]
    StepUnder1ns,
    /// `max_step_ps` is greater than `stop_ps`.
    StepOverStop,
    /// `grid_ps` is below 1000.
    #[serde(rename = "grid_under_1ns")]
    GridUnder1ns,
    /// `grid_ps` is greater than `stop_ps`.
    GridOverStop,
}

/// Which rule a schedule event broke. A switch, a `steps` source, a potentiometer, a
/// `serial` source, and a device event such as a `cst816` touch share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRule {
    /// The event is at t = 0, where the operating point already holds `initial`.
    AtZero,
    /// The event does not change the value in force.
    SameState,
    /// The event is not strictly after the previous event.
    NotIncreasing,
    /// The event is less than `MIN_GAP_PS` after the previous event's last possible edge.
    TooClose,
    /// The event ends past the largest `u64` picosecond. For a switch that is the end of its
    /// bounce window.
    Overflow,
    /// The event starts before the previous serial event ends.
    Overlap,
}

/// Which rule a source waveform's time broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveformRule {
    /// The first jump is at t = 0, where the operating point already holds `initial`.
    AtZero,
    /// Two jumps are less than `MIN_GAP_PS` apart.
    TooClose,
    /// The first fall is past the largest `u64` picosecond.
    Overflow,
    /// `mark` and `space` are the same SI value, so the line never changes.
    Equal,
}

/// Which rule of its protocol a chip saw broken. The chip drops what broke it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolRule {
    /// A command the model does not run: a command word cut short by a release of CS, phases
    /// past the command word, an opcode other than the one that command takes, data lanes that
    /// do not match that opcode, too few parameter bytes, or a window past the panel.
    Command,
    /// Pixels while the pixel format is not 16 bits.
    PixelFormat,
    /// An I2C target's SDA change came due while SCL was high, so SDA stayed.
    SdaWhileSclHigh,
    /// A bit whose high or low sat outside the timing windows.
    Bit,
    /// A read that came sooner than the sensor answers one: inside its warm-up after power-on,
    /// or before its minimum interval after the last read it answered.
    Interval,
    /// `CSB` was low, so the part is in SPI mode and does not answer on I2C.
    Spi,
    /// `spi3w_en` was set. 3-wire SPI is not modelled.
    Spi3w,
    /// A `config` (0xF5) write arrived in normal mode and would have changed `config`, so it was
    /// ignored (BST-BMP280-DS001 §4.3.5).
    ConfigInNormal,
    /// The supply is up, and under 4.5 V. The shaft still moves, slower than at 4.8 V.
    Supply,
    /// The moving draw pulled an SG90's supply under 3.0 V. The shaft holds.
    SupplySag,
    /// A scroll command, which the model does not run. The picture stays still.
    Scroll,
    /// A read transfer from a target that answers none.
    Read,
    /// An HD44780 operation attempted in 8-bit bus mode over a 4-bit PCF8574 backpack.
    BusWidth,
    /// A 5×10 font selection on a 5×8 character LCD.
    Font,
    /// An HD44780 command or data write arrived before its power-on reset wait elapsed (15 ms
    /// after VCC reaches 4.5 V, or 40 ms after 2.7 V).
    InitWait,
}

impl ProtocolRule {
    pub(crate) const ALL: [ProtocolRule; 15] = [
        ProtocolRule::Command,
        ProtocolRule::PixelFormat,
        ProtocolRule::SdaWhileSclHigh,
        ProtocolRule::Bit,
        ProtocolRule::Interval,
        ProtocolRule::Spi,
        ProtocolRule::Spi3w,
        ProtocolRule::ConfigInNormal,
        ProtocolRule::Supply,
        ProtocolRule::SupplySag,
        ProtocolRule::Scroll,
        ProtocolRule::Read,
        ProtocolRule::BusWidth,
        ProtocolRule::Font,
        ProtocolRule::InitWait,
    ];

    /// Its place in [`ALL`](Self::ALL).
    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The rule a device names in `Cx::broke`, by its name in `diagnostics.json`.
    pub(crate) fn named(name: &str) -> Option<ProtocolRule> {
        match name {
            "command" => Some(ProtocolRule::Command),
            "pixel_format" => Some(ProtocolRule::PixelFormat),
            "sda_while_scl_high" => Some(ProtocolRule::SdaWhileSclHigh),
            "bit" => Some(ProtocolRule::Bit),
            "interval" => Some(ProtocolRule::Interval),
            "spi" => Some(ProtocolRule::Spi),
            "spi3w" => Some(ProtocolRule::Spi3w),
            "config_in_normal" => Some(ProtocolRule::ConfigInNormal),
            "supply" => Some(ProtocolRule::Supply),
            "supply_sag" => Some(ProtocolRule::SupplySag),
            "scroll" => Some(ProtocolRule::Scroll),
            "read" => Some(ProtocolRule::Read),
            "bus_width" => Some(ProtocolRule::BusWidth),
            "font" => Some(ProtocolRule::Font),
            "init_wait" => Some(ProtocolRule::InitWait),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Diagnostic {
    /// JSON is malformed or does not match the v0 schema.
    NetlistSyntax {
        path: String,
        line: usize,
        column: usize,
        detail: String,
    },
    /// `version` is not one this engine reads. `supported` is every version it reads.
    NetlistVersion {
        found: u64,
        supported: &'static [u64],
    },
    /// A name breaks the grammar, or a part is named `net`.
    NetlistName {
        path: String,
        name: String,
        problem: NameError,
    },
    /// Two parts, or two nets, share a name.
    NetlistDuplicate { name: Ref },
    /// A pin names a net that is not in `nets`.
    NetlistUnknownNet { pin: Ref, net: String },
    /// A part's pin keys differ from its kind's pin set.
    NetlistPins {
        part: Ref,
        missing: Vec<&'static str>,
        unexpected: Vec<String>,
    },
    /// A value's `unit` is not the unit its field is measured in.
    NetlistUnit {
        part: Ref,
        field: Cow<'static, str>,
        expected: &'static str,
        found: String,
    },
    /// A value is outside its field's domain.
    NetlistValue {
        part: Ref,
        field: Cow<'static, str>,
        value: f64,
        rule: ValueRule,
    },
    /// A part's `firmware` is not a program for its flash. `line` is 1-based, `None` when no
    /// one line is at fault.
    NetlistFirmware {
        part: Ref,
        line: Option<usize>,
        rule: FirmwareRule,
    },
    /// A blob the part names in its `role` field is missing, or is not the blob it names.
    /// `bytes` is the netlist's length, and `found` the blob's when one was filed.
    NetlistBlob {
        part: Ref,
        role: Role,
        sha256: Sha256,
        rule: BlobRule,
        bytes: u64,
        found: Option<u64>,
        source: Source,
    },
    /// `pin` is on net `found`, and its kind ties it to `with`, on net `expected`.
    NetlistSameNet {
        pin: Ref,
        with: Ref,
        found: Name,
        expected: Name,
    },
    /// `ground` names a net that is not in `nets`.
    NetlistGround { net: String },
    /// A transient analysis field breaks a rule.
    NetlistAnalysis {
        field: &'static str,
        value: u64,
        rule: AnalysisRule,
    },
    /// `field[event]` breaks a schedule rule. `field` is `events`, `voltage.events`,
    /// `current.events`, or `position.events`. `end_ps` is the previous event's end. Only an
    /// `overlap` message reads either one. `data` does not hold them.
    NetlistSchedule {
        part: Ref,
        field: &'static str,
        event: usize,
        rule: ScheduleRule,
        end_ps: u64,
    },
    /// A world quantity's name breaks a rule. `part` is the part whose field reads it, or `None`
    /// for a name in `world`.
    NetlistWorld {
        part: Option<Ref>,
        field: Cow<'static, str>,
        name: String,
        rule: WorldRule,
    },
    /// A source waveform's time field breaks a rule. `field` is owned when it names one event.
    NetlistWaveform {
        part: Ref,
        field: Cow<'static, str>,
        value: u64,
        rule: WaveformRule,
    },
    /// The net has no DC path to ground. Its voltage is held only by gmin.
    NetFloating { net: Ref },
    /// A voltage source's `p` and `n` are both on `net`, so the circuit shorts it.
    NetShort { part: Ref, net: Ref },
    /// The MNA matrix is singular at `unknown`.
    SolverSingular { at: Time, unknown: Ref },
    /// A solution component or an observed quantity is not finite.
    SolverNonFinite { at: Time, unknown: Ref },
    /// The dense system has more unknowns than it will allocate.
    SolverTooLarge {
        at: Time,
        unknowns: usize,
        limit: usize,
    },
    /// The run accepted `limit` steps and had not reached its stop.
    SolverStepLimit { at: Time, limit: u64 },
    /// The linear solves toward the step after `at`, the last accepted step, took more than
    /// `limit` multiply-adds.
    SolverStepWork { at: Time, limit: u64 },
    /// The chips kept changing state for `rounds` solves at one time. `chips` changed in the last.
    ChipUnsettled {
        at: Time,
        chips: Vec<Ref>,
        rounds: u32,
    },
    /// A chip saw `rule` broken, first at `at`.
    ChipProtocol {
        part: Ref,
        at: Time,
        rule: ProtocolRule,
    },
    /// A chip whose CS and SCLK sit on `sender`'s nets dropped an SPI transaction whose data
    /// lanes are not on its own data pins, first at `at`.
    ChipLinkMiswired { part: Ref, at: Time, sender: Name },
    /// A chip's `peripheral` sent a `kind` record on `pin`, whose net also reaches `other`,
    /// which does not take that record kind on that pin. It ends the run.
    ChipLinkUnsupported {
        pin: Ref,
        other: Ref,
        at: Time,
        peripheral: &'static str,
        kind: LinkKind,
    },
    /// `pin` floats at `at`: it is an input with its pull-up off, no DC path joins `net` to a
    /// supply or to ground but the chip's own leakage, and its firmware has taken in its level
    /// since it last became such an input, alone or with the rest of its port. Once per pin.
    ChipFloatingInput { pin: Ref, net: Ref, at: Time },
    /// A chip's core reached `op` at `pc`, which the model does not run there. `reason` says
    /// what is missing. It ends the run.
    ChipUnsupportedInstruction {
        part: Ref,
        at: Time,
        op: &'static str,
        reason: Unsupported,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's core fetched code at `pc`, where the model maps no memory. It ends the run.
    ChipUnmappedFetch {
        part: Ref,
        at: Time,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's core reached a jump to itself at `pc` that nothing can take it out of: every
    /// enabled interrupt at or below the core's level, no source on the NMI, and no watchdog
    /// running. It ends the run.
    ChipHalted {
        part: Ref,
        at: Time,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware started a `peripheral` transfer from the store at `pc` with `setting`,
    /// or set `setting` on `peripheral` there, which the model does not run. It ends the run.
    ChipUnsupportedSetting {
        part: Ref,
        at: Time,
        peripheral: &'static str,
        setting: Setting,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A stage of a chip's watchdog ran out and reset `reset`, the CPU, the system or the RTC
    /// domain. It ends the run.
    ChipWatchdog {
        part: Ref,
        at: Time,
        watchdog: &'static str,
        stage: u8,
        reset: &'static str,
    },
    /// A chip's firmware started an eFuse program command from the store at `pc`, which would
    /// burn eFuses. It ends the run.
    ChipEfuseProgram {
        part: Ref,
        at: Time,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware sent its flash `command` from the store at `pc`, which the model
    /// refuses for `reason`. `command` is `None` when no one command byte went. It ends the run.
    ChipFlashCommand {
        part: Ref,
        at: Time,
        command: Option<u8>,
        reason: FlashRefusal,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware picked a CPU clock the model does not run from the store at `pc`. It
    /// ends the run.
    ChipUnsupportedClock {
        part: Ref,
        at: Time,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware reset the chip from the store at `pc`. The model does not run the boot
    /// ROM, so the chip stays in reset to the end of the run.
    ChipRestart {
        part: Ref,
        at: Time,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware reset the chip from the store at `pc` after recording `reason`, or no
    /// reason the model can read. It ends the run.
    ChipFaultRestart {
        part: Ref,
        at: Time,
        reason: ResetReason,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's core loaded from `address` from `pc`, in a cache window where the MMU maps no
    /// flash. It ends the run.
    ChipUnmappedLoad {
        part: Ref,
        at: Time,
        address: u32,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
    },
    /// A chip's firmware read or wrote `register` at `address`, which the model keeps no state
    /// for, first at `at` from `pc`, with a write when `write` is set. A read gives 0 and a
    /// write is dropped.
    ChipUnmodeledRegister {
        part: Ref,
        at: Time,
        address: u32,
        register: String,
        pc: u32,
        /// The APP CPU ran the code at `pc`, rather than the PRO CPU.
        app: bool,
        symbol: Option<String>,
        write: bool,
        /// The register keeps the write, and the model does not run what it does.
        kept: bool,
    },
    /// The grid gives more rows than the trace holds, found before the first solve, when
    /// `at` is `None`. The test-only step-rows mode can also fill the trace and refuse the
    /// next step. Then `at` is that step, and the rows already written stay.
    TraceTooLarge {
        rows: u64,
        limit: usize,
        columns: usize,
        at: Option<Time>,
    },
    /// The row at `at` did not solve, twice, so it is the line between the accepted steps at
    /// `from` and `to`.
    TraceRowInterpolated { at: Time, from: Time, to: Time },
    /// Newton found no operating point.
    SolverConvergence {
        at: Time,
        /// The junction that failed worst in the last iteration, else the unknown.
        unknown: Ref,
        /// Newton iterations over every attempt.
        iterations: u32,
        /// Smallest extra node conductance that converged, in siemens.
        gmin: Option<f64>,
        /// Largest source scale that converged, from 0 to 1.
        source_scale: f64,
        solve: Solve,
    },
    /// A rated current went over its limit.
    RatingCurrent(Excursion),
    /// A rated power went over its limit.
    RatingPower(Excursion),
    /// A chip pin's voltage left its rated window.
    RatingVoltage(VoltageExcursion),
    /// A chip's supply pin dropped between unpowered and its datasheet minimum.
    RatingSupplyLow(SupplyExcursion),
    /// A linear regulator was held at its current limit and its output sagged.
    RegulatorCurrentLimit(CurrentLimitExcursion),
}

/// Which record kind [`Diagnostic::ChipLinkUnsupported`] could not route faithfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Spi,
    Pulses,
}

/// The solve that did not converge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solve {
    /// The operating point, or the restart solve after an edge.
    Ladder,
    /// A time step, halved down to `step_ps`.
    Step { step_ps: u64 },
    /// A probe locating a chip event, `step_ps` after the last accepted time.
    Probe { step_ps: u64 },
}

/// One rated quantity over its limit, merged over the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct Excursion {
    /// A part quantity such as `#<part>.i` or `#<part>.p`, which is also the observation key,
    /// or a chip pin, `#<part>.pin.<pin>`, whose current the run works out from its stage.
    pub quantity: Ref,
    /// The rating, in SI units.
    pub limit: f64,
    /// Largest magnitude seen. For a sustained rating, the largest reading of its filter.
    pub peak: f64,
    pub peak_at: Time,
    /// First sample over the limit.
    pub first: Time,
    /// Total time over the limit. `None` for an operating point, which has no duration.
    pub over_ps: Option<u64>,
    /// How the datasheet means the rating: a peak, checked on each sample, or sustained, read
    /// through a filter with its time constant.
    pub basis: Basis,
    /// The quantity's own magnitude at `peak_at`. For a peak rating it is `peak`.
    pub sample: f64,
    /// `Some` when the finding is a sustained rating over now: `sample` is the latest sample,
    /// over the rating, and the filter, `peak`, is still under it. The value is how long that
    /// sample would hold before the filter crosses. `None` for a filter that has crossed.
    pub held_ps: Option<NonZeroU64>,
}

/// One chip pin outside its rated voltage window, merged over the whole run. Volts are over
/// the chip's ground pin.
#[derive(Clone, Debug, PartialEq)]
pub struct VoltageExcursion {
    /// `#<part>.pin.<pin>`.
    pub pin: Ref,
    /// The window's floor at the peak, or `None` when the part rates no floor.
    pub low: Option<f64>,
    /// The window's ceiling at the peak, or `None` when the part rates no ceiling.
    pub high: Option<f64>,
    /// The pin's voltage where it was farthest outside the window.
    pub peak: f64,
    pub peak_at: Time,
    /// First sample outside the window.
    pub first: Time,
    /// Total time outside the window. `None` for an operating point, which has no duration.
    pub over_ps: Option<u64>,
    /// The peak is the drive across the part, and `low`/`high` are its operating window.
    /// A chip pin's window leaves this false.
    pub operating: bool,
}

/// One chip supply pin between unpowered and its datasheet minimum, merged over the whole run.
/// Volts are over the chip's ground pin.
#[derive(Clone, Debug, PartialEq)]
pub struct SupplyExcursion {
    /// `#<part>.pin.<pin>`.
    pub pin: Ref,
    /// The datasheet minimum supply voltage, in V.
    pub min: f64,
    /// Clock frequency in Hz when the minimum depends on the chip's clock (such as an ATmega328P).
    pub clock_hz: Option<u32>,
    /// Lowest supply voltage held continuously for at least `SUPPLY_HOLD_PS` while above the
    /// unpowered floor and below the minimum (or the lowest bound held across the episode when no
    /// warning episode lasted `SUPPLY_HOLD_PS`, or the operating-point sample), in V.
    pub voltage: f64,
    /// When the lowest held supply voltage occurred.
    pub peak_at: Time,
    /// First sample below the minimum.
    pub first: Time,
    /// Total time below the minimum. `None` for an operating point, which has no duration.
    pub over_ps: Option<u64>,
}

/// One linear regulator held at its current limit with its output pulled below regulation,
/// merged over the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentLimitExcursion {
    /// `#<part>.pin.out`.
    pub pin: Ref,
    /// `current_limit`, in A.
    pub limit: f64,
    /// Nominal `output_voltage`, in V.
    pub target: f64,
    /// Lowest output voltage over `gnd` held for at least `LIMIT_HOLD_PS` while clamped, in V.
    pub voltage: f64,
    /// Pass current delivered at `peak_at`, in A.
    pub current: f64,
    /// When the lowest held output voltage occurred.
    pub peak_at: Time,
    /// First sample held at the current limit.
    pub first: Time,
    /// Total time held at the current limit. `None` for an operating point, which has no duration.
    pub over_ps: Option<u64>,
}

impl Diagnostic {
    pub fn code(&self) -> &'static str {
        match self {
            Diagnostic::NetlistSyntax { .. } => "netlist.syntax",
            Diagnostic::NetlistVersion { .. } => "netlist.version",
            Diagnostic::NetlistName { .. } => "netlist.name",
            Diagnostic::NetlistDuplicate { .. } => "netlist.duplicate",
            Diagnostic::NetlistUnknownNet { .. } => "netlist.unknown_net",
            Diagnostic::NetlistPins { .. } => "netlist.pins",
            Diagnostic::NetlistUnit { .. } => "netlist.unit",
            Diagnostic::NetlistValue { .. } => "netlist.value",
            Diagnostic::NetlistFirmware { .. } => "netlist.firmware",
            Diagnostic::NetlistBlob { .. } => "netlist.blob",
            Diagnostic::NetlistSameNet { .. } => "netlist.same_net",
            Diagnostic::NetlistGround { .. } => "netlist.ground",
            Diagnostic::NetlistAnalysis { .. } => "netlist.analysis",
            Diagnostic::NetlistSchedule { .. } => "netlist.schedule",
            Diagnostic::NetlistWaveform { .. } => "netlist.waveform",
            Diagnostic::NetlistWorld { .. } => "netlist.world",
            Diagnostic::NetFloating { .. } => "net.floating",
            Diagnostic::NetShort { .. } => "net.short",
            Diagnostic::SolverSingular { .. } => "solver.singular",
            Diagnostic::SolverNonFinite { .. } => "solver.nonfinite",
            Diagnostic::SolverTooLarge { .. } => "solver.too_large",
            Diagnostic::SolverStepLimit { .. } => "solver.step_limit",
            Diagnostic::SolverStepWork { .. } => "solver.step_work",
            Diagnostic::SolverConvergence { .. } => "solver.convergence",
            Diagnostic::ChipUnsettled { .. } => "chip.unsettled",
            Diagnostic::ChipProtocol { .. } => "chip.protocol",
            Diagnostic::ChipLinkMiswired { .. } => "chip.link_miswired",
            Diagnostic::ChipLinkUnsupported { .. } => "chip.link_unsupported",
            Diagnostic::ChipFloatingInput { .. } => "chip.floating_input",
            Diagnostic::ChipUnsupportedInstruction { .. } => "chip.unsupported_instruction",
            Diagnostic::ChipUnmappedFetch { .. } => "chip.unmapped_fetch",
            Diagnostic::ChipHalted { .. } => "chip.halted",
            Diagnostic::ChipUnsupportedSetting { .. } => "chip.unsupported_setting",
            Diagnostic::ChipWatchdog { .. } => "chip.watchdog",
            Diagnostic::ChipEfuseProgram { .. } => "chip.efuse_program",
            Diagnostic::ChipFlashCommand { .. } => "chip.flash_command",
            Diagnostic::ChipUnsupportedClock { .. } => "chip.unsupported_clock",
            Diagnostic::ChipRestart { .. } => "chip.restart",
            Diagnostic::ChipFaultRestart { .. } => "chip.fault_restart",
            Diagnostic::ChipUnmappedLoad { .. } => "chip.unmapped_load",
            Diagnostic::ChipUnmodeledRegister { .. } => "chip.unmodeled_register",
            Diagnostic::TraceTooLarge { .. } => "trace.too_large",
            Diagnostic::TraceRowInterpolated { .. } => "trace.row_interpolated",
            Diagnostic::RatingCurrent(_) => "rating.current",
            Diagnostic::RatingPower(_) => "rating.power",
            Diagnostic::RatingVoltage(_) => "rating.voltage",
            Diagnostic::RatingSupplyLow(_) => "rating.supply_low",
            Diagnostic::RegulatorCurrentLimit(_) => "regulator.current_limit",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::NetFloating { .. }
            | Diagnostic::ChipProtocol { .. }
            | Diagnostic::ChipLinkMiswired { .. }
            | Diagnostic::ChipFloatingInput { .. }
            | Diagnostic::ChipUnmodeledRegister { .. }
            | Diagnostic::ChipRestart { .. }
            | Diagnostic::TraceRowInterpolated { .. }
            | Diagnostic::RatingCurrent(_)
            | Diagnostic::RatingPower(_)
            | Diagnostic::RatingVoltage(_)
            | Diagnostic::RatingSupplyLow(_)
            | Diagnostic::RegulatorCurrentLimit(_) => Severity::Warning,
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistFirmware { .. }
            | Diagnostic::NetlistBlob { .. }
            | Diagnostic::NetlistSameNet { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. }
            | Diagnostic::NetlistSchedule { .. }
            | Diagnostic::NetlistWaveform { .. }
            | Diagnostic::NetlistWorld { .. }
            | Diagnostic::NetShort { .. }
            | Diagnostic::SolverSingular { .. }
            | Diagnostic::SolverNonFinite { .. }
            | Diagnostic::SolverTooLarge { .. }
            | Diagnostic::SolverStepLimit { .. }
            | Diagnostic::SolverStepWork { .. }
            | Diagnostic::SolverConvergence { .. }
            | Diagnostic::ChipUnsettled { .. }
            | Diagnostic::ChipLinkUnsupported { .. }
            | Diagnostic::ChipUnsupportedInstruction { .. }
            | Diagnostic::ChipUnmappedFetch { .. }
            | Diagnostic::ChipHalted { .. }
            | Diagnostic::ChipUnsupportedSetting { .. }
            | Diagnostic::ChipWatchdog { .. }
            | Diagnostic::ChipEfuseProgram { .. }
            | Diagnostic::ChipFlashCommand { .. }
            | Diagnostic::ChipUnsupportedClock { .. }
            | Diagnostic::ChipFaultRestart { .. }
            | Diagnostic::ChipUnmappedLoad { .. }
            | Diagnostic::TraceTooLarge { .. } => Severity::Error,
        }
    }

    pub fn at(&self) -> Option<Time> {
        match self {
            Diagnostic::SolverSingular { at, .. }
            | Diagnostic::SolverNonFinite { at, .. }
            | Diagnostic::SolverTooLarge { at, .. }
            | Diagnostic::SolverStepLimit { at, .. }
            | Diagnostic::SolverStepWork { at, .. }
            | Diagnostic::SolverConvergence { at, .. }
            | Diagnostic::ChipUnsettled { at, .. }
            | Diagnostic::ChipProtocol { at, .. }
            | Diagnostic::ChipLinkMiswired { at, .. }
            | Diagnostic::ChipLinkUnsupported { at, .. }
            | Diagnostic::ChipFloatingInput { at, .. }
            | Diagnostic::ChipUnsupportedInstruction { at, .. }
            | Diagnostic::ChipUnmappedFetch { at, .. }
            | Diagnostic::ChipHalted { at, .. }
            | Diagnostic::ChipUnsupportedSetting { at, .. }
            | Diagnostic::ChipWatchdog { at, .. }
            | Diagnostic::ChipEfuseProgram { at, .. }
            | Diagnostic::ChipFlashCommand { at, .. }
            | Diagnostic::ChipUnsupportedClock { at, .. }
            | Diagnostic::ChipRestart { at, .. }
            | Diagnostic::ChipFaultRestart { at, .. }
            | Diagnostic::ChipUnmappedLoad { at, .. }
            | Diagnostic::ChipUnmodeledRegister { at, .. }
            | Diagnostic::TraceRowInterpolated { at, .. } => Some(*at),
            Diagnostic::TraceTooLarge { at, .. } => *at,
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                Some(excursion.first)
            }
            Diagnostic::RatingVoltage(excursion) => Some(excursion.first),
            Diagnostic::RatingSupplyLow(excursion) => Some(excursion.first),
            Diagnostic::RegulatorCurrentLimit(excursion) => Some(excursion.first),
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistFirmware { .. }
            | Diagnostic::NetlistBlob { .. }
            | Diagnostic::NetlistSameNet { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. }
            | Diagnostic::NetlistSchedule { .. }
            | Diagnostic::NetlistWaveform { .. }
            | Diagnostic::NetlistWorld { .. }
            | Diagnostic::NetFloating { .. }
            | Diagnostic::NetShort { .. } => None,
        }
    }

    pub fn refs(&self) -> Vec<Ref> {
        match self {
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. } => Vec::new(),
            Diagnostic::NetlistDuplicate { name } => vec![name.clone()],
            Diagnostic::NetlistUnknownNet { pin, .. } => vec![pin.clone()],
            Diagnostic::NetlistSameNet { pin, with, .. } => vec![pin.clone(), with.clone()],
            Diagnostic::NetlistPins { part, .. }
            | Diagnostic::NetlistUnit { part, .. }
            | Diagnostic::NetlistValue { part, .. }
            | Diagnostic::NetlistFirmware { part, .. }
            | Diagnostic::NetlistBlob { part, .. }
            | Diagnostic::NetlistSchedule { part, .. }
            | Diagnostic::NetlistWaveform { part, .. } => vec![part.clone()],
            Diagnostic::NetlistWorld { part, .. } => part.iter().cloned().collect(),
            Diagnostic::NetFloating { net } => vec![net.clone()],
            Diagnostic::ChipFloatingInput { pin, net, .. } => vec![pin.clone(), net.clone()],
            Diagnostic::ChipLinkUnsupported { pin, other, .. } => vec![pin.clone(), other.clone()],
            Diagnostic::NetShort { part, net } => vec![part.clone(), net.clone()],
            Diagnostic::SolverSingular { unknown, .. }
            | Diagnostic::SolverNonFinite { unknown, .. }
            | Diagnostic::SolverConvergence { unknown, .. } => vec![unknown.clone()],
            Diagnostic::ChipUnsettled { chips, .. } => chips.clone(),
            Diagnostic::ChipProtocol { part, .. }
            | Diagnostic::ChipLinkMiswired { part, .. }
            | Diagnostic::ChipUnsupportedInstruction { part, .. }
            | Diagnostic::ChipUnmappedFetch { part, .. }
            | Diagnostic::ChipHalted { part, .. }
            | Diagnostic::ChipUnsupportedSetting { part, .. }
            | Diagnostic::ChipWatchdog { part, .. }
            | Diagnostic::ChipEfuseProgram { part, .. }
            | Diagnostic::ChipFlashCommand { part, .. }
            | Diagnostic::ChipUnsupportedClock { part, .. }
            | Diagnostic::ChipRestart { part, .. }
            | Diagnostic::ChipFaultRestart { part, .. }
            | Diagnostic::ChipUnmappedLoad { part, .. }
            | Diagnostic::ChipUnmodeledRegister { part, .. } => vec![part.clone()],
            Diagnostic::SolverTooLarge { .. }
            | Diagnostic::SolverStepLimit { .. }
            | Diagnostic::SolverStepWork { .. }
            | Diagnostic::TraceTooLarge { .. }
            | Diagnostic::TraceRowInterpolated { .. } => Vec::new(),
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                vec![excursion.quantity.clone()]
            }
            Diagnostic::RatingVoltage(excursion) => vec![excursion.pin.clone()],
            Diagnostic::RatingSupplyLow(excursion) => vec![excursion.pin.clone()],
            Diagnostic::RegulatorCurrentLimit(excursion) => vec![excursion.pin.clone()],
        }
    }

    /// Whether this is a sustained rating over now, whose filter has not crossed.
    pub fn over_now(&self) -> bool {
        matches!(
            self,
            Diagnostic::RatingCurrent(Excursion {
                held_ps: Some(_),
                ..
            }) | Diagnostic::RatingPower(Excursion {
                held_ps: Some(_),
                ..
            })
        )
    }

    /// Whether this is a peak rating's warning. A sustained rating with a peak limit warns on
    /// the same quantity under both bases.
    pub fn peak(&self) -> bool {
        matches!(
            self,
            Diagnostic::RatingCurrent(Excursion {
                basis: Basis::Peak,
                ..
            }) | Diagnostic::RatingPower(Excursion {
                basis: Basis::Peak,
                ..
            })
        )
    }

    /// `(time, code, refs)`. `Results` sorts by this and then by JSON bytes.
    pub fn sort_key(&self) -> (Option<Time>, &'static str, Vec<Ref>) {
        (self.at(), self.code(), self.refs())
    }

    /// Each ref in its own text, as a run folder prints it.
    fn message(&self) -> String {
        self.message_with(&|r| r.to_string())
    }

    /// The same text, with each ref named by `namer`.
    pub fn message_with(&self, namer: &dyn Fn(&Ref) -> String) -> String {
        match self {
            Diagnostic::NetlistSyntax { path, detail, .. } => {
                format!("schema mismatch at `{path}` ({detail})")
            }
            Diagnostic::NetlistVersion { found, supported } => {
                let supported: Vec<_> = supported.iter().map(u64::to_string).collect();
                format!(
                    "netlist version {found} is not supported; this engine reads {}",
                    supported.join(", ")
                )
            }
            Diagnostic::NetlistName {
                path,
                name,
                problem,
            } => {
                format!(
                    "name `{name}` at `{path}` is not valid ({})",
                    name_problem(*problem)
                )
            }
            Diagnostic::NetlistDuplicate { name } => {
                let name = namer(name);
                format!("{name} is declared more than once")
            }
            Diagnostic::NetlistUnknownNet { pin, net } => {
                let pin = namer(pin);
                format!("pin {pin} names net `{net}`, which is not in `nets`")
            }
            Diagnostic::NetlistPins { part, .. } => {
                let part = namer(part);
                format!("{part} pins do not match its kind")
            }
            Diagnostic::NetlistUnit {
                part,
                field,
                expected,
                found,
            } => {
                let part = namer(part);
                format!(
                    "{part} {field} is in `{found}`; it must be in `{expected}` (SI base units)"
                )
            }
            Diagnostic::NetlistValue {
                part,
                field,
                value,
                rule,
            } => {
                let world = match part {
                    Ref::World(quantity) => *quantity,
                    _ => crate::world::Quantity::Temperature,
                };
                let part = namer(part);
                let byte = *value as u8;
                let count = *value as u16;
                let value = number_text(*value);
                match rule {
                    ValueRule::Positive => {
                        format!("{part} {field} is {value}; it must be positive")
                    }
                    ValueRule::FiniteReciprocal => {
                        format!("{part} {field} is {value}; its reciprocal must be finite")
                    }
                    ValueRule::FiniteJunction => {
                        let requirement = if field.as_ref() == "zener_voltage" {
                            "Isb must be normal and positive, and vcrit must be finite and positive"
                        } else if field.as_ref() == "zener_impedance" {
                            "nbvt must be normal"
                        } else {
                            "n*VT must be normal, and vcrit must be finite and positive"
                        };
                        format!("{part} {field} is {value}; {requirement}")
                    }
                    ValueRule::NonNegative => {
                        format!("{part} {field} is {value}; it must not be negative")
                    }
                    ValueRule::BounceRoom => format!(
                        "{part} {field} is {value}; it must fit 2 * max_bounces edges {} ps apart",
                        MIN_GAP_PS
                    ),
                    ValueRule::Fraction => {
                        format!("{part} {field} is {value}; it must be between 0 and 1")
                    }
                    ValueRule::BelowOne => {
                        format!("{part} {field} is {value}; it must be at least 0 and below 1")
                    }
                    ValueRule::AboveHigh => {
                        format!("{part} {field} is {value}; it must not be above high")
                    }
                    ValueRule::Hysteresis => {
                        format!("{part} {field} is {value}; it must be below pull_in_current")
                    }
                    ValueRule::AboveResistance => format!(
                        "{part} {field} is {value}; it must exceed series_resistance + breakdown_resistance"
                    ),
                    ValueRule::AboveDrop => format!(
                        "{part} {field} is {value}; it must exceed zener_current * (series_resistance + breakdown_resistance)"
                    ),
                    ValueRule::Unsupported => {
                        format!("{part} {field} is 0x{byte:02X}; the model does not run that mode")
                    }
                    ValueRule::Span => {
                        format!("{part} {field} is {value}; it must be from 0 to 4095")
                    }
                    ValueRule::Leds => {
                        format!("{part} {field} is {count}; it must be from 1 to 1024")
                    }
                    ValueRule::Delay => {
                        format!("{part} {field} is {value}; it must be at most 1000000")
                    }
                    ValueRule::Range => {
                        let row = world.row();
                        format!(
                            "{part} {field} is {value} {}; {} must be from {} to {} {}",
                            row.unit,
                            row.name,
                            number_text(row.min),
                            number_text(row.max),
                            row.unit
                        )
                    }
                    ValueRule::Finite => {
                        format!("{part} {field} is {value}; it must be finite")
                    }
                    ValueRule::Distinct => {
                        format!("{part} {field} is {value}; it must differ from x0")
                    }
                }
            }
            Diagnostic::NetlistFirmware { part, line, rule } => {
                let part = namer(part);
                let what = match rule {
                    FirmwareRule::Record => "is not an Intel HEX record",
                    FirmwareRule::Checksum => "has a bad checksum",
                    FirmwareRule::RecordType => "has a record type other than 00 to 05",
                    FirmwareRule::AfterEnd => "comes after the end-of-file record",
                    FirmwareRule::NoEnd => "has no end-of-file record",
                    FirmwareRule::Empty => "has no data record, so the flash would be erased",
                    FirmwareRule::PastFlash => "puts a byte past the end of the flash",
                    FirmwareRule::Region => {
                        "places a flash region past the end of the 16 MB flash or onto a byte an earlier region placed"
                    }
                    FirmwareRule::Image => {
                        "has no app the bootloader would start: a partition table, an app partition, blank otadata, and an ESP32-S3 app image whose checksum and hash hold"
                    }
                    FirmwareRule::Rom => {
                        "needs `rom` with `flash` and none with `elf`, and `rom` must be the revision 0 ROM, sha256 c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd, 949552 bytes"
                    }
                    FirmwareRule::Overlap if line.is_none() => {
                        "loads two segments onto one byte of memory"
                    }
                    FirmwareRule::Overlap => "places a byte an earlier record placed",
                    FirmwareRule::Elf => "is not a 32-bit little-endian Xtensa ELF executable",
                    FirmwareRule::Unmapped => {
                        "loads a segment, or starts, outside the RAM a program may load into"
                    }
                };
                match line {
                    Some(line) => format!("{part} firmware line {line} {what}"),
                    None => format!("{part} firmware {what}"),
                }
            }
            Diagnostic::NetlistBlob {
                part,
                role,
                sha256,
                rule,
                bytes,
                found,
                source,
            } => {
                let part = namer(part);
                let role = role.as_str();
                let found = found.unwrap_or(0);
                match (source, rule) {
                    (Source::Folders, BlobRule::Missing) => format!(
                        "{part} {role} blob {sha256} is not in the blobs folder beside the netlist or in any --blobs folder"
                    ),
                    (Source::Folders, BlobRule::Hash) => format!(
                        "{part} {role} blob {sha256}: no file under that name in the blobs folder beside the netlist or in a --blobs folder has that SHA-256"
                    ),
                    (Source::Folders, BlobRule::Length) => format!(
                        "{part} {role} blob {sha256} in the blobs folder beside the netlist or a --blobs folder has {found} bytes; the netlist says {bytes}"
                    ),
                    (Source::List, BlobRule::Missing) => {
                        format!("{part} {role} blob {sha256} is not in the blob list")
                    }
                    (Source::List, BlobRule::Hash) => format!(
                        "{part} {role} blob {sha256}: the blob under that name in the blob list has another SHA-256"
                    ),
                    (Source::List, BlobRule::Length) => format!(
                        "{part} {role} blob {sha256} in the blob list has {found} bytes; the netlist says {bytes}"
                    ),
                }
            }
            Diagnostic::NetlistSameNet {
                pin,
                with,
                found,
                expected,
            } => {
                let pin = namer(pin);
                let with = namer(with);
                format!(
                    "pin {pin} is on `{}`; it must be on `{}`, as {with} is",
                    found.as_str(),
                    expected.as_str()
                )
            }
            Diagnostic::NetlistGround { net } => format!("ground `{net}` is not in `nets`"),
            Diagnostic::NetlistAnalysis { field, value, rule } => {
                let must = match rule {
                    AnalysisRule::Positive => "it must be positive",
                    AnalysisRule::StepUnder1ns | AnalysisRule::GridUnder1ns => {
                        "it must be at least 1000"
                    }
                    AnalysisRule::StepOverStop | AnalysisRule::GridOverStop => {
                        "it must not exceed stop_ps"
                    }
                };
                format!("analysis {field} is {value}; {must}")
            }
            Diagnostic::NetlistSchedule {
                part,
                field,
                event,
                rule,
                end_ps,
            } => {
                // A switch's events have a bounce window. A world quantity's are levels.
                let switch = *field == "events" && !matches!(part, Ref::World(_));
                let part = namer(part);
                let what = match rule {
                    ScheduleRule::AtZero => {
                        "is at 0 ps; an event must come after t = 0".to_string()
                    }
                    ScheduleRule::SameState if switch => "does not change the contact".to_string(),
                    ScheduleRule::SameState => "does not change the level".to_string(),
                    ScheduleRule::NotIncreasing => "is not after the previous event".to_string(),
                    ScheduleRule::TooClose if switch => format!(
                        "is less than {} ps after the previous event and its bounce window",
                        MIN_GAP_PS
                    ),
                    ScheduleRule::TooClose => {
                        format!("is less than {} ps after the previous event", MIN_GAP_PS)
                    }
                    ScheduleRule::Overflow if switch => {
                        "has a bounce window that ends past the largest time".to_string()
                    }
                    ScheduleRule::Overflow => "ends past the largest time".to_string(),
                    ScheduleRule::Overlap => {
                        format!("starts before the previous event ends at {end_ps} ps")
                    }
                };
                format!("{part} {field}[{event}] {what}")
            }
            Diagnostic::NetlistWorld {
                part,
                field,
                name,
                rule,
            } => {
                let at = match part {
                    Some(part) => format!("{} {field}", namer(part)),
                    None => field.to_string(),
                };
                match rule {
                    WorldRule::Unknown => {
                        let names: Vec<&str> = crate::world::Quantity::ALL
                            .iter()
                            .map(|q| q.name())
                            .collect();
                        format!(
                            "{at} names `{name}`, which is not a world quantity; the quantities are {}",
                            names.join(", ")
                        )
                    }
                    WorldRule::Undeclared => {
                        format!("{at} reads {name}, which the netlist's `world` does not declare")
                    }
                    WorldRule::Temperature => {
                        format!("{at} is a B equation, which reads temperature, not {name}")
                    }
                    WorldRule::NonNegative => {
                        format!(
                            "{at} is a power curve, which reads a quantity that does not go below 0, not {name}"
                        )
                    }
                }
            }
            Diagnostic::NetlistWaveform {
                part,
                field,
                value,
                rule,
            } => {
                let part = namer(part);
                match rule {
                    WaveformRule::Equal => {
                        let source = field.strip_suffix(".space").unwrap_or(field.as_ref());
                        format!("{part} {source}.mark and {source}.space are equal")
                    }
                    WaveformRule::AtZero => {
                        format!("{part} {field} is {value}; a jump must come after t = 0")
                    }
                    WaveformRule::TooClose => format!(
                        "{part} {field} is {value}; two jumps must be at least {} ps apart",
                        MIN_GAP_PS
                    ),
                    WaveformRule::Overflow => {
                        format!(
                            "{part} {field} is {value}; the first fall is past the largest time"
                        )
                    }
                }
            }
            Diagnostic::NetFloating { net } => {
                let net = namer(net);
                format!("{net} has no DC path to ground")
            }
            Diagnostic::NetShort { part, net } => {
                let part = namer(part);
                let net = namer(net);
                format!(
                    "{part} holds `p` and `n` apart, but both are on {net}, so the circuit shorts it"
                )
            }
            Diagnostic::SolverSingular { unknown, .. } => {
                let unknown = namer(unknown);
                format!("singular matrix at {unknown}")
            }
            Diagnostic::SolverNonFinite { unknown, .. } => {
                let unknown = namer(unknown);
                format!("non-finite number at {unknown}")
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => format!("{unknowns} unknowns exceeds the solver limit of {limit}"),
            Diagnostic::SolverStepLimit { limit, .. } => format!(
                "the run accepted {limit} steps, its limit; dense switch or source schedules cost steps, not rows"
            ),
            Diagnostic::SolverStepWork { limit, .. } => format!(
                "one step took more than {limit} solver operations, its limit; the circuit is too large or too stiff to step"
            ),
            Diagnostic::SolverConvergence {
                at,
                unknown,
                iterations,
                source_scale,
                solve: Solve::Ladder,
                ..
            } if *at > Time::ZERO => {
                let unknown = namer(unknown);
                format!(
                    "Newton did not converge at {unknown} after {iterations} iterations in the restart solve at {} ps; source stepping reached {source_scale}",
                    at.as_ps()
                )
            }
            Diagnostic::SolverConvergence {
                unknown,
                iterations,
                source_scale,
                solve: Solve::Ladder,
                ..
            } => {
                let unknown = namer(unknown);
                format!(
                    "Newton did not converge at {unknown} after {iterations} iterations; source stepping reached {source_scale}"
                )
            }
            Diagnostic::SolverConvergence {
                unknown,
                iterations,
                solve: Solve::Step { step_ps },
                ..
            } => {
                let unknown = namer(unknown);
                format!(
                    "Newton did not converge at {unknown} after {iterations} iterations; the time step halved to {step_ps} ps"
                )
            }
            Diagnostic::SolverConvergence {
                at,
                unknown,
                iterations,
                solve: Solve::Probe { .. },
                ..
            } => {
                let unknown = namer(unknown);
                format!(
                    "Newton did not converge at {unknown} after {iterations} iterations in the probe at {} ps locating a chip event",
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnsettled { at, rounds, .. } => format!(
                "the chips kept changing state for {rounds} solves at {} ps",
                at.as_ps()
            ),
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::SdaWhileSclHigh,
            } => {
                let part = namer(part);
                format!(
                    "{part} left SDA unmoved because SCL was high at {} ps, breaking rule {}, the first time",
                    at.as_ps(),
                    rule_text(&ProtocolRule::SdaWhileSclHigh)
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Bit,
            } => {
                let part = namer(part);
                format!(
                    "{part} dropped a bit that broke rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Bit),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Interval,
            } => {
                let part = namer(part);
                format!(
                    "{part} did not answer a read that broke rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Interval),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Spi,
            } => {
                let part = namer(part);
                format!(
                    "{part} did not answer on I2C because CSB was low, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Spi),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Spi3w,
            } => {
                let part = namer(part);
                format!(
                    "{part} was set to 3-wire SPI, which is not modelled, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Spi3w),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::ConfigInNormal,
            } => {
                let part = namer(part);
                format!(
                    "{part} ignored a write to config (0xF5) in normal mode (BST-BMP280-DS001 §4.3.5; write sleep mode to ctrl_meas 0xF4 before config), breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::ConfigInNormal),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Supply,
            } => {
                let part = namer(part);
                format!(
                    "{part} is below 4.5 V at {} ps, so it moves slower, breaking rule {}, the first time",
                    at.as_ps(),
                    rule_text(&ProtocolRule::Supply)
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::SupplySag,
            } => {
                let part = namer(part);
                format!(
                    "{part} supply sags under the servo's moving draw at {} ps, so the shaft holds, breaking rule {}, the first time",
                    at.as_ps(),
                    rule_text(&ProtocolRule::SupplySag)
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Scroll,
            } => {
                let part = namer(part);
                format!(
                    "{part} did not run a scroll command, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Scroll),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Read,
            } => {
                let part = namer(part);
                format!(
                    "{part} answered a read with 0xFF bytes, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Read),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::BusWidth,
            } => {
                let part = namer(part);
                format!(
                    "{part} dropped an 8-bit bus operation on a 4-bit backpack, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::BusWidth),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::Font,
            } => {
                let part = namer(part);
                format!(
                    "{part} does not run the 5x10 font, breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::Font),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol {
                part,
                at,
                rule: ProtocolRule::InitWait,
            } => {
                let part = namer(part);
                format!(
                    "{part} dropped a write before the HD44780U power-on reset wait elapsed (15 ms after VCC reaches 4.5 V, or 40 ms after 2.7 V), breaking rule {} at {} ps, the first time",
                    rule_text(&ProtocolRule::InitWait),
                    at.as_ps()
                )
            }
            Diagnostic::ChipProtocol { part, at, rule } => {
                let part = namer(part);
                format!(
                    "{part} dropped a transfer that broke rule {} at {} ps, the first time",
                    rule_text(rule),
                    at.as_ps()
                )
            }
            Diagnostic::ChipFloatingInput { pin, net, at } => {
                let pin = namer(pin);
                let net = namer(net);
                format!(
                    "{pin} floats on {net} at {} ps, and the firmware has taken in its level since the pin last became an input with its pull-up off, alone or with the rest of its port: there is no DC path to a supply or to ground, and a capacitor does not hold the pin's level, so the level is whatever leakage and noise leave; add a pull-up or a pull-down resistor, or turn the pin's pull-up on",
                    at.as_ps()
                )
            }
            Diagnostic::ChipLinkMiswired { part, at, sender } => {
                let part = namer(part);
                format!(
                    "{part} dropped an SPI transaction from #{} whose data lanes are not on its own data pins at {} ps, the first time",
                    sender.as_str(),
                    at.as_ps()
                )
            }
            Diagnostic::ChipLinkUnsupported {
                pin,
                other,
                at,
                peripheral,
                kind,
            } => {
                let pin = namer(pin);
                let other = namer(other);
                match kind {
                    LinkKind::Spi => format!(
                        "{pin} sent an SPI record from {peripheral} at {} ps, but its net reaches {other}, which does not take SPI records there; the model sends {peripheral} output as records rather than pin edges and runs hardware SPI only into parts that take SPI directly, so use shiftOut or bit-banged SPI instead",
                        at.as_ps()
                    ),
                    LinkKind::Pulses => format!(
                        "{pin} sent a pulse record from {peripheral} at {} ps, but its net reaches {other}, which does not take pulse records there; the model sends {peripheral} output as records rather than pin edges, so the data line must be wired straight to the pixel's data input",
                        at.as_ps()
                    ),
                }
            }
            Diagnostic::ChipUnsupportedInstruction {
                part,
                at,
                op,
                reason,
                pc,
                app,
                symbol,
            } => {
                let what = match reason {
                    Unsupported::Op if *op == "unknown" => {
                        "a word that decodes to no instruction".to_string()
                    }
                    Unsupported::Op => format!("`{op}`"),
                    Unsupported::Unaligned => format!("an unaligned `{op}`"),
                    Unsupported::Litbase => "`l32r` with LITBASE".to_string(),
                    Unsupported::LoopEnd => format!("`{op}` as the last op of a loop"),
                };
                let part = namer(part);
                format!(
                    "{part} reached {what} at {pc:#010x}{}{}, which the model does not run, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnmappedFetch {
                part,
                at,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} fetched code at {pc:#010x}{}{}, where the model maps no memory, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipHalted {
                part,
                at,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} halted at {pc:#010x}{}{}, a jump to itself that no interrupt or watchdog can leave, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnsupportedSetting {
                part,
                at,
                peripheral,
                setting,
                pc,
                app,
                symbol,
            } => {
                let what = match (*peripheral, *setting) {
                    ("AC", Setting::Register("aco_read")) => {
                        "read the AC output while the comparator is on".to_string()
                    }
                    ("CPU", Setting::Sleep) => {
                        "put the chip to sleep in a reserved sleep mode".to_string()
                    }
                    ("RTC_CNTL", Setting::Register("pin_hold")) => {
                        "enabled pin hold (gpio_hold_en)".to_string()
                    }
                    (_, Setting::Transfer(setting)) => {
                        format!("started a {peripheral} transfer with setting {setting}")
                    }
                    (_, Setting::Interrupt) => format!("enabled a {peripheral} interrupt"),
                    (_, Setting::Register(setting)) => format!("set {peripheral} {setting}"),
                    (_, Setting::Unsimulated(what)) => format!("used {peripheral}, {what},"),
                    (_, Setting::Sleep) => "put the chip to sleep".to_string(),
                };
                let why = match (*peripheral, *setting) {
                    (_, Setting::Interrupt) => "which the model never raises",
                    (_, Setting::Unsimulated(_)) => "which the model does not simulate",
                    ("AC", Setting::Register("aco_read")) => {
                        "which the model works out from the logic level on each input pin, not its voltage"
                    }
                    ("GDMA", Setting::Register("reset_while_sending")) => {
                        "which the model does not run; ESP-IDF sets GDMA's reset when it restarts or panics, so the console holds any panic text"
                    }
                    _ => "which the model does not run",
                };
                let part = namer(part);
                format!(
                    "{part} {what} at {pc:#010x}{}{}, {why}, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipWatchdog {
                part,
                at,
                watchdog,
                stage,
                reset,
            } => {
                let part = namer(part);
                format!(
                    "{part} {watchdog} watchdog's stage {stage} ran out and reset the {reset} at {} ps; the firmware neither fed nor disabled it",
                    at.as_ps()
                )
            }
            Diagnostic::ChipEfuseProgram {
                part,
                at,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} started an eFuse program command at {pc:#010x}{}{}, which would burn eFuses, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipFlashCommand {
                part,
                at,
                command,
                reason,
                pc,
                app,
                symbol,
            } => {
                let command = match command {
                    Some(command) => format!("command 0x{command:02X}"),
                    None => "a command".to_string(),
                };
                let (what, why) = match reason {
                    FlashRefusal::Unsupported => (
                        format!("sent the flash {command}"),
                        "which the model does not run",
                    ),
                    FlashRefusal::Write => (
                        format!("wrote the flash with {command}"),
                        "which the model does not simulate; Wi-Fi, Bluetooth and saved settings (Preferences, EEPROM, NVS) write the flash",
                    ),
                    FlashRefusal::StatusWrite => (
                        format!("sent the flash the status write {command}"),
                        "which could lock the flash",
                    ),
                    FlashRefusal::NoCommandPhase => (
                        "sent the flash a user command with no command phase".to_string(),
                        "which the model does not run",
                    ),
                    FlashRefusal::StartBits => (
                        "started more than one SPI1 command in one write".to_string(),
                        "which the model does not run",
                    ),
                    FlashRefusal::PastEnd => (
                        format!("sent the flash the read {command}"),
                        "which reaches past the flash's end",
                    ),
                    FlashRefusal::AddressBits => (
                        format!("sent the flash {command}"),
                        "whose address is longer than 32 bits",
                    ),
                    FlashRefusal::Spi0 => (
                        "started a command on SPI0".to_string(),
                        "which the model does not run",
                    ),
                    FlashRefusal::Pads => (
                        "took a flash pad from SPI0 and SPI1".to_string(),
                        "which could send the flash any command",
                    ),
                    FlashRefusal::Cs => (
                        format!("sent the flash {command} with CS0 disabled or CS1 enabled"),
                        "which the model does not run",
                    ),
                };
                let part = namer(part);
                format!(
                    "{part} {what} at {pc:#010x}{}{}, {why}, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnsupportedClock {
                part,
                at,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} picked a CPU clock the model does not run, the RC fast clock or a reserved setting, at {pc:#010x}{}{} at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipRestart {
                part,
                at,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} reset itself at {pc:#010x}{}{} at {} ps; the model does not run the boot ROM, so the chip stays in reset",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipFaultRestart {
                part,
                at,
                reason,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} reset itself at {pc:#010x}{}{} {}, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    match reason {
                        ResetReason::Panic => "after a panic",
                        ResetReason::InterruptWatchdog => "after its interrupt watchdog fired",
                        ResetReason::TaskWatchdog => "after its task watchdog fired",
                        ResetReason::Watchdog => "after a watchdog fired",
                        ResetReason::Brownout => "after a brownout",
                        ResetReason::Unreadable => "with no reset reason the model can read",
                    },
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnmappedLoad {
                part,
                at,
                address,
                pc,
                app,
                symbol,
            } => {
                let part = namer(part);
                format!(
                    "{part} loaded from {address:#010x} at {pc:#010x}{}{}, where the MMU maps no flash, at {} ps",
                    symbol_text(symbol),
                    core_text(*app),
                    at.as_ps()
                )
            }
            Diagnostic::ChipUnmodeledRegister {
                part,
                at,
                address,
                register,
                pc,
                app,
                symbol,
                write,
                kept,
            } => {
                let part = namer(part);
                format!(
                    "{part} {} {register} at {address:#010x}, {} {} ps from {pc:#010x}{}{}",
                    if *write { "wrote" } else { "read" },
                    if *kept {
                        "whose effect the model does not run, at"
                    } else {
                        "which the model does not keep, first at"
                    },
                    at.as_ps(),
                    symbol_text(symbol),
                    core_text(*app)
                )
            }
            Diagnostic::TraceTooLarge {
                rows,
                limit,
                columns,
                at: None,
            } => format!(
                "the grid gives {rows} rows; the trace holds {limit} rows of {columns} columns; raise grid_ps or shorten stop_ps"
            ),
            Diagnostic::TraceTooLarge {
                limit,
                at: Some(at),
                ..
            } => format!(
                "the trace holds {limit} rows; the step at {} ps did not fit",
                at.as_ps()
            ),
            Diagnostic::TraceRowInterpolated { at, from, to } => format!(
                "the row at {} ps did not solve; it is the line between the steps at {} ps and {} ps",
                at.as_ps(),
                from.as_ps(),
                to.as_ps()
            ),
            Diagnostic::RatingCurrent(excursion) => over_rating(excursion, "A", namer),
            Diagnostic::RatingPower(excursion) => over_rating(excursion, "W", namer),
            Diagnostic::RatingVoltage(excursion) => outside_window(excursion, namer),
            Diagnostic::RatingSupplyLow(excursion) => supply_low(excursion, namer),
            Diagnostic::RegulatorCurrentLimit(excursion) => regulator_limit(excursion, namer),
        }
    }

    fn data(&self) -> serde_json::Value {
        let mut data = self.fields();
        if self.on_app_cpu() {
            data["core"] = serde_json::json!("app");
        }
        data
    }

    /// Whether the code a chip diagnostic names ran on the APP CPU.
    fn on_app_cpu(&self) -> bool {
        matches!(
            self,
            Diagnostic::ChipUnsupportedInstruction { app: true, .. }
                | Diagnostic::ChipUnmappedFetch { app: true, .. }
                | Diagnostic::ChipHalted { app: true, .. }
                | Diagnostic::ChipUnsupportedSetting { app: true, .. }
                | Diagnostic::ChipEfuseProgram { app: true, .. }
                | Diagnostic::ChipFlashCommand { app: true, .. }
                | Diagnostic::ChipUnsupportedClock { app: true, .. }
                | Diagnostic::ChipRestart { app: true, .. }
                | Diagnostic::ChipFaultRestart { app: true, .. }
                | Diagnostic::ChipUnmappedLoad { app: true, .. }
                | Diagnostic::ChipUnmodeledRegister { app: true, .. }
        )
    }

    fn fields(&self) -> serde_json::Value {
        match self {
            Diagnostic::NetlistSyntax {
                path,
                line,
                column,
                detail,
            } => serde_json::json!({
                "path": path,
                "line": line,
                "column": column,
                "detail": detail,
            }),
            Diagnostic::NetlistVersion { found, supported } => {
                serde_json::json!({ "found": found, "supported": supported })
            }
            Diagnostic::NetlistName {
                path,
                name,
                problem,
            } => {
                serde_json::json!({ "path": path, "name": name, "problem": problem })
            }
            Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetFloating { .. }
            | Diagnostic::NetShort { .. }
            | Diagnostic::ChipFloatingInput { .. } => {
                serde_json::json!({})
            }
            Diagnostic::NetlistUnknownNet { net, .. } => serde_json::json!({ "net": net }),
            Diagnostic::NetlistPins {
                missing,
                unexpected,
                ..
            } => serde_json::json!({ "missing": missing, "unexpected": unexpected }),
            Diagnostic::NetlistUnit {
                field,
                expected,
                found,
                ..
            } => serde_json::json!({
                "field": field,
                "expected": expected,
                "found": found,
            }),
            Diagnostic::NetlistValue {
                field, value, rule, ..
            } => serde_json::json!({ "field": field, "value": value, "rule": rule }),
            Diagnostic::NetlistFirmware { line, rule, .. } => {
                serde_json::json!({ "field": "firmware", "line": line, "rule": rule })
            }
            Diagnostic::NetlistBlob {
                role,
                sha256,
                rule,
                bytes,
                found,
                ..
            } => serde_json::json!({
                "bytes": bytes,
                "found": found,
                "role": role.as_str(),
                "rule": rule,
                "sha256": sha256.to_string(),
            }),
            Diagnostic::NetlistSameNet {
                found, expected, ..
            } => serde_json::json!({ "expected": expected.as_str(), "found": found.as_str() }),
            Diagnostic::NetlistGround { net } => serde_json::json!({ "net": net }),
            Diagnostic::NetlistAnalysis { field, value, rule } => {
                serde_json::json!({ "field": field, "rule": rule, "value": value })
            }
            Diagnostic::NetlistSchedule { event, rule, .. } => {
                serde_json::json!({ "event": event, "rule": rule })
            }
            Diagnostic::NetlistWaveform {
                field, value, rule, ..
            } => serde_json::json!({ "field": field, "rule": rule, "value": value }),
            Diagnostic::NetlistWorld {
                field, name, rule, ..
            } => serde_json::json!({ "field": field, "name": name, "rule": rule }),
            Diagnostic::SolverSingular { .. } | Diagnostic::SolverNonFinite { .. } => {
                serde_json::json!({})
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => serde_json::json!({ "limit": limit, "unknowns": unknowns }),
            Diagnostic::SolverStepLimit { limit, .. }
            | Diagnostic::SolverStepWork { limit, .. } => {
                serde_json::json!({ "limit": limit })
            }
            Diagnostic::SolverConvergence {
                iterations,
                gmin,
                source_scale,
                solve,
                ..
            } => {
                let mut data = serde_json::json!({
                    "gmin": gmin,
                    "iterations": iterations,
                    "source_scale": source_scale,
                });
                if let Solve::Step { step_ps } | Solve::Probe { step_ps } = solve {
                    data["step_ps"] = serde_json::json!(step_ps);
                }
                data
            }
            Diagnostic::ChipUnsettled { rounds, .. } => serde_json::json!({ "rounds": rounds }),
            Diagnostic::ChipProtocol { rule, .. } => serde_json::json!({ "rule": rule }),
            Diagnostic::ChipLinkMiswired { sender, .. } => {
                serde_json::json!({ "sender": sender.as_str() })
            }
            Diagnostic::ChipLinkUnsupported {
                peripheral, kind, ..
            } => {
                serde_json::json!({ "kind": kind, "peripheral": peripheral })
            }
            Diagnostic::ChipUnsupportedInstruction {
                op,
                reason,
                pc,
                symbol,
                ..
            } => {
                serde_json::json!({ "op": op, "pc": pc, "reason": reason.name(), "symbol": symbol })
            }
            Diagnostic::ChipUnmappedFetch { pc, symbol, .. }
            | Diagnostic::ChipHalted { pc, symbol, .. } => {
                serde_json::json!({ "pc": pc, "symbol": symbol })
            }
            Diagnostic::ChipUnsupportedSetting {
                peripheral,
                setting,
                pc,
                symbol,
                ..
            } => serde_json::json!({
                "peripheral": peripheral,
                "pc": pc,
                "setting": setting.name(),
                "symbol": symbol,
            }),
            Diagnostic::ChipWatchdog {
                watchdog,
                stage,
                reset,
                ..
            } => {
                serde_json::json!({ "watchdog": watchdog, "stage": stage, "reset": reset })
            }
            Diagnostic::ChipEfuseProgram { pc, symbol, .. }
            | Diagnostic::ChipUnsupportedClock { pc, symbol, .. }
            | Diagnostic::ChipRestart { pc, symbol, .. } => {
                serde_json::json!({ "pc": pc, "symbol": symbol })
            }
            Diagnostic::ChipFlashCommand {
                command,
                reason,
                pc,
                symbol,
                ..
            } => serde_json::json!({
                "command": command,
                "pc": pc,
                "reason": reason.name(),
                "symbol": symbol,
            }),
            Diagnostic::ChipFaultRestart {
                reason, pc, symbol, ..
            } => serde_json::json!({ "pc": pc, "reason": reason.name(), "symbol": symbol }),
            Diagnostic::ChipUnmappedLoad {
                address,
                pc,
                symbol,
                ..
            } => serde_json::json!({ "address": address, "pc": pc, "symbol": symbol }),
            Diagnostic::ChipUnmodeledRegister {
                address,
                register,
                pc,
                symbol,
                write,
                kept,
                ..
            } => serde_json::json!({
                "address": address,
                "kept": kept,
                "pc": pc,
                "register": register,
                "symbol": symbol,
                "write": write,
            }),
            Diagnostic::TraceTooLarge {
                rows,
                limit,
                columns,
                ..
            } => serde_json::json!({ "columns": columns, "limit": limit, "rows": rows }),
            Diagnostic::TraceRowInterpolated { from, to, .. } => {
                serde_json::json!({ "from_ps": from, "to_ps": to })
            }
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                let (tau_ps, average) = match excursion.basis {
                    Basis::Peak => (None, None),
                    Basis::Sustained { tau_ps, average } => (Some(tau_ps), Some(average.as_str())),
                };
                serde_json::json!({
                    "average": average,
                    "held_ps": excursion.held_ps,
                    "limit": excursion.limit,
                    "now": excursion.held_ps.is_some(),
                    "over_ps": excursion.over_ps,
                    "peak": excursion.peak,
                    "peak_ps": excursion.peak_at,
                    "sample": excursion.sample,
                    "tau_ps": tau_ps,
                })
            }
            Diagnostic::RatingVoltage(excursion) => {
                serde_json::json!({
                    "high": excursion.high,
                    "low": excursion.low,
                    "over_ps": excursion.over_ps,
                    "peak": excursion.peak,
                    "peak_ps": excursion.peak_at,
                })
            }
            Diagnostic::RatingSupplyLow(excursion) => {
                let mut data = serde_json::json!({
                    "limit": excursion.min,
                    "over_ps": excursion.over_ps,
                    "peak_ps": excursion.peak_at,
                    "voltage": excursion.voltage,
                });
                if let Some(hz) = excursion.clock_hz {
                    data["clock_hz"] = serde_json::json!(hz);
                }
                data
            }
            Diagnostic::RegulatorCurrentLimit(excursion) => {
                serde_json::json!({
                    "current": excursion.current,
                    "limit": excursion.limit,
                    "over_ps": excursion.over_ps,
                    "peak_ps": excursion.peak_at,
                    "target": excursion.target,
                    "voltage": excursion.voltage,
                })
            }
        }
    }
}

/// ` on the APP CPU` for code the APP CPU ran, or nothing.
fn core_text(app: bool) -> &'static str {
    if app { " on the APP CPU" } else { "" }
}

/// ` in name+0x1c`, or nothing with no symbol.
fn symbol_text(symbol: &Option<String>) -> String {
    symbol
        .as_ref()
        .map(|name| format!(" in {name}"))
        .unwrap_or_default()
}

/// The rule as `data.rule` names it.
fn rule_text(rule: &ProtocolRule) -> String {
    serde_json::to_value(rule)
        .ok()
        .and_then(|name| name.as_str().map(str::to_string))
        .expect("a rule serializes to its name")
}

/// The same characters `data.value` uses. `Display` for a subnormal is hundreds of digits. A
/// value that is not finite, which `data.value` holds as `null`, is `NaN`, `inf` or `-inf`.
pub(crate) fn number_text(value: f64) -> String {
    if value.is_finite() {
        serde_json::to_string(&value).expect("a finite number serializes")
    } else {
        value.to_string()
    }
}

fn over_rating(excursion: &Excursion, unit: &str, namer: &dyn Fn(&Ref) -> String) -> String {
    let Excursion {
        quantity,
        limit,
        peak,
        over_ps,
        basis,
        sample,
        held_ps,
        ..
    } = excursion;
    let quantity = namer(quantity);
    let value = |v: f64| format!("{} {unit}", significant_text(v));
    let rating = format!("over its {} rating{}", value(*limit), for_time(*over_ps));
    match (basis, held_ps) {
        (_, Some(held)) => format!(
            "{quantity} is {} now, over its {} rating; it warns if held about {} ps",
            value(*sample),
            value(*limit),
            significant_ps(held.get())
        ),
        (Basis::Sustained { tau_ps, average }, None) => format!(
            "{quantity} {} with a {} ms time constant{}, {rating}",
            sustained_text(*average, value(*peak)),
            significant_text(*tau_ps as f64 / 1e9),
            while_it_is(value(*sample), value(*peak))
        ),
        (Basis::Peak, None) => format!("{quantity} peaks at {}, {rating}", value(*peak)),
    }
}

/// `averages 3 W`, or `carries 43.4 mA RMS`: a sustained rating's filter reading.
pub fn sustained_text(average: Average, reading: String) -> String {
    match average {
        Average::Mean => format!("averages {reading}"),
        Average::Rms => format!("carries {reading} RMS"),
    }
}

/// `value` to 3 significant digits. A rating message prints its values so, and a value is
/// over its rating only when it is over after both are rounded.
pub fn significant(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    if !value.is_finite() {
        return value;
    }
    format!("{value:.2e}").parse().unwrap_or(value)
}

/// [`significant`] as a message prints it: the characters `data` uses, with no `.0`.
pub fn significant_text(value: f64) -> String {
    let text = number_text(significant(value));
    match text.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => text,
    }
}

/// Whether `value` is over `limit` once both are rounded to 3 significant digits, when `limit`
/// is already [`significant`].
///
/// Rounding to 3 significant digits moves a finite positive number by at most half a
/// percent, so more than 1% away from `limit` the comparison needs no formatting. Inside
/// that band, and when `limit` is not a positive finite number, the check rounds `value`.
#[inline]
pub(crate) fn exceeds_rounded(value: f64, limit: f64) -> bool {
    match decided_without_rounding(value, limit) {
        Some(over) => over,
        None => significant(value) > limit,
    }
}

/// `Some` when `value` and `limit` are far enough apart that rounding cannot change
/// [`exceeds_rounded`]. `None` when the exact rounding has to run.
fn decided_without_rounding(value: f64, limit: f64) -> Option<bool> {
    if !(value > 0.0 && limit > 0.0) {
        return None;
    }
    let ratio = value / limit;
    if ratio > 1.01 {
        Some(true)
    } else if ratio < 0.99 {
        Some(false)
    } else {
        None
    }
}

/// `ps` to 2 significant digits, for a time a message gives as "about".
pub fn significant_ps(ps: u64) -> u64 {
    let digits = ps.checked_ilog10().unwrap_or(0);
    let scale = 10u64.pow(digits.saturating_sub(1));
    (ps + scale / 2) / scale * scale
}

/// ` while it is {sample}` beside a window's reading, or nothing when the two print the same,
/// as at an operating point, whose DC value fills the window.
pub fn while_it_is(sample: String, reading: String) -> String {
    if sample == reading {
        String::new()
    } else {
        format!(" while it is {sample}")
    }
}

/// ` for {over_ps} ps` once a run has been over for some time. Nothing for an operating
/// point, which has no duration, or for a finding first over on the last accepted step.
fn for_time(over_ps: Option<u64>) -> String {
    match over_ps {
        Some(over) if over > 0 => format!(" for {over} ps"),
        _ => String::new(),
    }
}

fn outside_window(excursion: &VoltageExcursion, namer: &dyn Fn(&Ref) -> String) -> String {
    let VoltageExcursion {
        pin,
        low,
        high,
        peak,
        over_ps,
        operating,
        ..
    } = excursion;
    let pin = namer(pin);
    if *operating {
        let low = low.unwrap_or(0.0);
        let high = high.unwrap_or(0.0);
        return format!(
            "{pin} peaks at {} V, outside its {} V to {} V operating voltage{}",
            significant_text(*peak),
            significant_text(low),
            significant_text(high),
            for_time(*over_ps)
        );
    }
    let window = match (low, high) {
        (Some(low), Some(high)) => format!(
            "its {} V to {} V rating",
            significant_text(*low),
            significant_text(*high)
        ),
        (Some(low), None) => format!("its {} V floor", significant_text(*low)),
        (None, Some(high)) => format!("its {} V ceiling", significant_text(*high)),
        (None, None) => unreachable!("a window rates a floor or a ceiling"),
    };
    format!(
        "{pin} reaches {} V over the chip's ground, outside {window}{}",
        significant_text(*peak),
        for_time(*over_ps)
    )
}

fn frequency_text(hz: u32) -> String {
    let hz = f64::from(hz);
    if hz >= 1e6 {
        format!("{} MHz", significant_text(hz / 1e6))
    } else if hz >= 1e3 {
        format!("{} kHz", significant_text(hz / 1e3))
    } else {
        format!("{} Hz", significant_text(hz))
    }
}

fn supply_low(excursion: &SupplyExcursion, namer: &dyn Fn(&Ref) -> String) -> String {
    let SupplyExcursion {
        pin,
        min,
        clock_hz,
        voltage,
        over_ps,
        ..
    } = excursion;
    let pin = namer(pin);
    let at_clock = clock_hz
        .map(|hz| format!(" at {}", frequency_text(hz)))
        .unwrap_or_default();
    format!(
        "{pin} supply drops to {} V over the chip's ground, below its {} V minimum{at_clock}{}",
        significant_text(*voltage),
        significant_text(*min),
        for_time(*over_ps)
    )
}

fn regulator_limit(excursion: &CurrentLimitExcursion, namer: &dyn Fn(&Ref) -> String) -> String {
    let CurrentLimitExcursion {
        pin,
        limit,
        target,
        voltage,
        current,
        over_ps,
        ..
    } = excursion;
    let pin = namer(pin);
    format!(
        "{pin} regulator output can give {} A at {} V; the circuit asks for more, so it is giving {} A and has dropped as low as {} V{}",
        significant_text(*limit),
        significant_text(*target),
        significant_text(*current),
        significant_text(*voltage),
        for_time(*over_ps),
    )
}

fn name_problem(problem: NameError) -> &'static str {
    match problem {
        NameError::Empty => "it is empty",
        NameError::TooLong => "it is longer than 64 bytes",
        NameError::BadChar => "it must be an ASCII letter, then letters, digits, or underscore",
        NameError::Reserved => "`net` is reserved",
    }
}

impl Serialize for Diagnostic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            code: &'static str,
            severity: Severity,
            refs: Vec<Ref>,
            t_ps: Option<Time>,
            message: String,
            data: &'a serde_json::Value,
        }
        let data = self.data();
        Wire {
            code: self.code(),
            severity: self.severity(),
            refs: self.refs(),
            t_ps: self.at(),
            message: self.message(),
            data: &data,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rounded_limit_compares_what_the_message_prints_on_a_three_digit_tie() {
        // Each value sits on a tie at the third digit, where binary rounding decides it.
        let ties = [
            (0.1255, 0.125),
            (0.02005, 0.02),
            (1.005, 1.0),
            (2.675, 2.67),
        ];
        for (value, limit) in ties {
            for (value, limit) in [(value, limit), (limit, value), (-value, -limit)] {
                assert_eq!(
                    exceeds_rounded(value, significant(limit)),
                    significant(value) > significant(limit),
                    "{value} {limit}"
                );
            }
        }
        assert!(exceeds_rounded(0.1255, 0.125), "0.1255 prints as 0.126");
        assert!(!exceeds_rounded(0.02005, 0.02), "0.02005 prints as 0.02");
        let mut value = 1e-6;
        while value < 1e4 {
            let tie = significant(value) + 5e-3 * libm::pow(10.0, libm::floor(libm::log10(value)));
            assert_eq!(
                exceeds_rounded(tie, significant(value)),
                significant(tie) > significant(value),
                "{tie} {value}"
            );
            value *= 1.07;
        }
    }

    #[test]
    fn a_device_names_each_protocol_rule_as_diagnostics_json_does() {
        for rule in ProtocolRule::ALL {
            assert_eq!(ProtocolRule::named(&rule_text(&rule)), Some(rule));
        }
        assert_eq!(ProtocolRule::named("glitch"), None);
    }

    #[test]
    fn rating_voltage_message_rounds_values_and_keeps_data_unrounded() {
        let diag = Diagnostic::RatingVoltage(VoltageExcursion {
            pin: "#s3__mod__u1.pin.GPIO4".parse().unwrap(),
            low: Some(-0.3),
            high: Some(3.600019234965837),
            peak: 4.76227892536249,
            peak_at: Time::from_ps(1_000_000),
            first: Time::from_ps(1_000_000),
            over_ps: Some(4_562_490_689),
            operating: false,
        });
        let value = serde_json::to_value(&diag).unwrap();
        assert_eq!(
            value["message"],
            "#s3__mod__u1.pin.GPIO4 reaches 4.76 V over the chip's ground, outside its -0.3 V to 3.6 V rating for 4562490689 ps"
        );
        assert_eq!(value["data"]["peak"], 4.76227892536249);
        assert_eq!(value["data"]["high"], 3.600019234965837);
        assert_eq!(value["data"]["low"], -0.3);

        let zero_floor = Diagnostic::RatingVoltage(VoltageExcursion {
            pin: "#c1.pin.a".parse().unwrap(),
            low: Some(-0.0),
            high: None,
            peak: -0.0012345,
            peak_at: Time::ZERO,
            first: Time::ZERO,
            over_ps: None,
            operating: false,
        });
        assert_eq!(
            zero_floor.message(),
            "#c1.pin.a reaches -0.00123 V over the chip's ground, outside its 0 V floor"
        );
    }

    #[test]
    fn a_value_far_from_its_rounded_limit_is_over_exactly_when_rounding_says_so() {
        let mut limits = vec![
            0.0,
            -0.0,
            -1.0,
            1.0,
            f64::MIN_POSITIVE,
            1e-320,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for exp in -12..=12 {
            let scale = libm::pow(10.0, exp as f64);
            for digits in [100, 101, 105, 110, 150, 500, 990, 995, 999] {
                limits.push(digits as f64 * scale / 100.0);
                limits.push(-(digits as f64) * scale / 100.0);
            }
        }
        for digits in 100..=999 {
            limits.push(digits as f64 / 100.0);
        }
        for limit in limits {
            let rounded = significant(limit);
            let mut values = vec![
                0.0,
                -0.0,
                rounded,
                -rounded,
                f64::MIN_POSITIVE,
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ];
            if rounded.is_finite() && rounded != 0.0 {
                for factor in [
                    0.5, 0.97, 0.989, 0.99, 0.995, 1.0, 1.005, 1.009, 1.01, 1.011, 1.02, 2.0,
                ] {
                    values.push(rounded * factor);
                    values.push(-rounded.abs() * factor);
                }
            }
            for value in values {
                assert_eq!(
                    exceeds_rounded(value, rounded),
                    significant(value) > rounded,
                    "value {value:?} limit {limit:?} rounded {rounded:?}"
                );
            }
        }
    }
}
