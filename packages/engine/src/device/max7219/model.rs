//! The MAX7219 8-digit / 8×8 LED matrix display driver as a device. Maxim / Analog Devices
//! MAX7219/MAX7221 datasheet (19-4452; Rev 4; 7/03).
//!
//! Serial data at `DIN` shifts into a 16-bit shift register on every rising edge of `CLK`,
//! regardless of `LOAD` (p. 6). On the falling edge of `CLK`, bit 15 (`D15`) appears at `DOUT`
//! after `tDO` = 25 ns (p. 3, Fig. 1), so data entered at `DIN` reaches the next chip's `DIN`
//! 16 clock cycles later. On the rising edge of `LOAD` (`CS`), the 16-bit shift register latches
//! into the addressed register (Table 2, p. 7). When `CLK` and `LOAD` rise together, `CLK`
//! shifts first and `LOAD` latches the new 16-bit word (p. 6).
//!
//! Power-up state (p. 7): all control registers reset to 0x00 (shutdown mode, display test off,
//! decode mode off, scan limit 0, intensity 0), blanking the display. Below [`MIN_VCC_V`] (4.0 V,
//! Electrical Characteristics p. 2) the host releases `DOUT` and `ISET`, the chip ignores edges,
//! resets to power-up state, blanks the matrix and draws nothing.
//!
//! Peak segment current `ISEG` is set by the external resistor `RSET` between `V+` and `ISET`
//! per Table 11 (p. 11) and `VLED`, scaled by `(V+ - V_ISET) / (5.0 - V_ISET)` when `V+` differs
//! from 5 V. Average current per lit dot is `ISEG * duty / scan_digits`, where `duty` is Table 7
//! `(2 * intensity + 1) / 32` and `scan_digits` is Table 8 `scan_limit + 1` (or `31/32` and `8`
//! during display test, Table 10).

use orbe_model::{
    Cx, Device, Draw, Drawn, Drive, Drives, Error, Levels, Panel, Params, Settings, Sha256, Shows,
    State, Stop, Time, View,
};

/// 8 columns (DP, A, B, C, D, E, F, G left to right on FC-16 / LedControl) by 8 rows (DIG0–DIG7).
pub const WIDTH: u16 = 8;
pub const HEIGHT: u16 = 8;
/// One byte per dot (`gray8`), row-major, 0 = off, 1..=255 = relative visual brightness.
pub const FRAME_BYTES: usize = 64;

/// Watch order.
pub const WATCH_DIN: usize = 0;
pub const WATCH_CLK: usize = 1;
pub const WATCH_LOAD: usize = 2;
pub const WATCH_SUPPLY: usize = 3;
const LOGIC: usize = 3;

/// Output order: push-pull `DOUT`, then `ISET` current-sense clamp.
pub const OUTPUTS: usize = 2;
pub const OUTPUT_DOUT: usize = 0;
pub const OUTPUT_ISET: usize = 1;

/// Sample order: `V(VCC) - V(GND)`, then `V(ISET) - V(GND)`.
pub const SAMPLE_VCC: usize = 0;
pub const SAMPLE_ISET: usize = 1;

/// Electrical Characteristics p. 2: Operating Supply Voltage `V+` minimum (4.0 V to 5.5 V).
pub const MIN_VCC_V: f64 = 4.0;

/// Electrical Characteristics p. 3: Data-Out Propagation Delay `tDO`, 25 ns max (`CL = 50 pF`).
pub const T_DO_PS: u64 = 25_000;

/// `ISET` pin clamp voltage: at `V+ = 5.0 V`, `RSET = 9.53 kΩ` sets `ISET = 0.4 mA` (p. 2, p. 11),
/// so `V_ISET = 5.0 - 9.53k * 0.4m = 1.188 V`.
pub const ISET_DROP_V: f64 = 1.188;
/// Small series resistance on the `ISET` clamp so `I_ISET = (V_ISET - ISET_DROP_V) / ISET_RES_OHM`
/// recovers external `RSET = (V_VCC - V_ISET) / I_ISET` exactly from node voltages.
pub const ISET_RESISTANCE_OHM: f64 = 100.0;

/// Reference average dot current for full-scale (`255`) `gray8` frame value: 40 mA full-scale
/// segment current across 8 scanned digits (`5.0 mA/dot`).
pub const REF_DOT_A: f64 = 0.005;

/// Setting names in [`Panel::settings`], in declaration order.
pub const SETTINGS: [&str; 6] = [
    "decode_mode",
    "intensity",
    "scan_limit",
    "display_test",
    "iseg_10ua",
    "dot_ua",
];

/// Register addresses, Table 2 (p. 7), bits D11–D8.
const REG_NOOP: u8 = 0x0;
const REG_DIGIT0: u8 = 0x1;
const REG_DIGIT7: u8 = 0x8;
const REG_DECODE: u8 = 0x9;
const REG_INTENSITY: u8 = 0xA;
const REG_SCAN_LIMIT: u8 = 0xB;
const REG_SHUTDOWN: u8 = 0xC;
const REG_DISPLAY_TEST: u8 = 0xF;

/// Table 5 (p. 8): Code B Font, bit order `D7=DP, D6=A, D5=B, D4=C, D3=D, D2=E, D1=F, D0=G`.
const CODE_B: [u8; 16] = [
    0x7E, // 0: A B C D E F
    0x30, // 1: B C
    0x6D, // 2: A B D E G
    0x79, // 3: A B C D G
    0x33, // 4: B C F G
    0x5B, // 5: A C D F G
    0x5F, // 6: A C D E F G
    0x70, // 7: A B C
    0x7F, // 8: A B C D E F G
    0x7B, // 9: A B C D F G
    0x01, // -: G
    0x4F, // E: A D E F G
    0x37, // H: B C E F G
    0x0E, // L: D E F
    0x67, // P: A B E F G
    0x00, // blank
];

/// Table 11 (p. 11): `RSET` (ohms) vs. Segment Current (`ISEG`) and LED Forward Voltage (`VLED`).
const TABLE11_VLED: [f64; 5] = [1.5, 2.0, 2.5, 3.0, 3.5];
const TABLE11_ISEG_A: [f64; 4] = [0.040, 0.030, 0.020, 0.010];
const TABLE11_RSET_OHM: [[f64; 5]; 4] = [
    [12_200.0, 11_800.0, 11_000.0, 10_600.0, 9_690.0], // 40 mA
    [17_800.0, 17_100.0, 15_800.0, 15_000.0, 14_000.0], // 30 mA
    [29_800.0, 28_000.0, 25_900.0, 24_500.0, 22_600.0], // 20 mA
    [66_700.0, 63_700.0, 59_300.0, 55_400.0, 51_200.0], // 10 mA
];

/// Minimum `RSET` clamped for numerical stability when `ISET` is shorted to `VCC`.
const MIN_RSET_OHM: f64 = 100.0;

/// Supply draw parameters, in SI units, passed from the part file.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawSpec {
    shutdown_a: f64,
    logic_a: f64,
    holds_above_v: f64,
}

/// MAX7219 state.
#[derive(Clone, Debug)]
pub struct Max7219 {
    /// 16-bit serial shift register; bit 15 is `D15` (oldest), bit 0 is `D0` (newest).
    shift: u16,
    /// Digit 0..7 registers (addresses 0x1..=0x8).
    digits: [u8; 8],
    /// Register 0x9: bit `d` selects Code B decode for digit `d`.
    decode_mode: u8,
    /// Register 0xA (bits 3..0): PWM duty = `(2 * intensity + 1) / 32`.
    intensity: u8,
    /// Register 0xB (bits 2..0): digits `0..=scan_limit` are scanned (`N = scan_limit + 1`).
    scan_limit: u8,
    /// Register 0xC bit 0 == 0: shutdown mode blanks display while retaining registers.
    shutdown: bool,
    /// Register 0xF bit 0 == 1: display test overrides all controls, lights all 64 LEDs at 31/32.
    display_test: bool,
    /// Current logical level driven on `DOUT`.
    dout: bool,
    /// Pending `DOUT` transition scheduled for `t + tDO` after a `CLK` falling edge.
    dout_due: Option<(Time, bool)>,
    /// Previous logic levels on `[DIN, CLK, LOAD]`, `None` while unpowered or before first input.
    prev: Option<[bool; LOGIC]>,
    /// Whether `V(VCC) - V(GND) >= MIN_VCC_V`.
    powered: bool,
    /// Last sampled `V(VCC) - V(GND)` in volts.
    supply_v: f64,
    /// Peak segment current `ISEG` in amperes, derived from `RSET` and `VLED`.
    i_seg_a: f64,
    /// Average forward current of each lit dot in amperes (`0.0` when blanked).
    dot_a: f64,
    /// Number of currently lit dots (`0..=64`).
    lit_dots: u8,
    /// LED forward voltage `VLED` in volts, for Table 11 lookup.
    vled_v: f64,
    /// Supply draw parameters, `None` when the part leaves `draw` out.
    draw: Option<DrawSpec>,
    /// Current 8×8 `gray8` frame buffer (row 0..7, col 0..7 = DP, A..G).
    frame: [u8; FRAME_BYTES],
}

impl State for Max7219 {}

/// Peak segment current `ISEG` (amperes) at `V+ = 5.0 V` for external `rset_ohm` and `vled_v`,
/// interpolating Table 11 (p. 11) linearly in `VLED` and log-log piecewise in `RSET`.
pub fn table11_iseg(rset_ohm: f64, vled_v: f64) -> f64 {
    if !rset_ohm.is_finite() || rset_ohm <= 0.0 {
        return 0.0;
    }
    let r = rset_ohm.max(MIN_RSET_OHM);
    // Interpolate the four RSET values at this VLED (clamped to Table 11's 1.5–3.5 V span).
    let mut r_at_vled = [0.0; 4];
    let v = vled_v.clamp(TABLE11_VLED[0], TABLE11_VLED[TABLE11_VLED.len() - 1]);
    let seg = (0..TABLE11_VLED.len() - 1)
        .find(|&k| v <= TABLE11_VLED[k + 1])
        .unwrap_or(TABLE11_VLED.len() - 2);
    let t = (v - TABLE11_VLED[seg]) / (TABLE11_VLED[seg + 1] - TABLE11_VLED[seg]);
    for row in 0..4 {
        let r0 = TABLE11_RSET_OHM[row][seg];
        let r1 = TABLE11_RSET_OHM[row][seg + 1];
        r_at_vled[row] = r0 + t * (r1 - r0);
    }
    // Below 10 mA (RSET > R_10mA), scale proportionally with 1 / RSET down to 0 A at open circuit.
    if r >= r_at_vled[3] {
        return TABLE11_ISEG_A[3] * (r_at_vled[3] / r);
    }
    // Piecewise linear interpolation in conductance (1 / RSET) between Table 11 rows
    // (exact at every Table 11 point, using only IEEE-754 arithmetic).
    let (i0, i1, r0, r1) = if r <= r_at_vled[1] {
        (
            TABLE11_ISEG_A[0],
            TABLE11_ISEG_A[1],
            r_at_vled[0],
            r_at_vled[1],
        )
    } else if r <= r_at_vled[2] {
        (
            TABLE11_ISEG_A[1],
            TABLE11_ISEG_A[2],
            r_at_vled[1],
            r_at_vled[2],
        )
    } else {
        (
            TABLE11_ISEG_A[2],
            TABLE11_ISEG_A[3],
            r_at_vled[2],
            r_at_vled[3],
        )
    };
    let (g, g0, g1) = (1.0 / r, 1.0 / r0, 1.0 / r1);
    i0 + (i1 - i0) * ((g - g0) / (g1 - g0))
}

impl Max7219 {
    fn reset_registers(&mut self) {
        self.shift = 0;
        self.digits = [0; 8];
        self.decode_mode = 0;
        self.intensity = 0;
        self.scan_limit = 0;
        self.shutdown = true;
        self.display_test = false;
        self.prev = None;
    }

    fn later(at: Time, delay: u64) -> Time {
        Time::from_ps(at.as_ps().saturating_add(delay))
    }

    /// The 8 segment bits (`D7=DP, D6=A, ..., D0=G`) for row `digit`.
    fn row_bits(&self, digit: usize) -> u8 {
        if self.display_test {
            return 0xFF;
        }
        if digit > usize::from(self.scan_limit) {
            return 0x00;
        }
        let raw = self.digits[digit];
        if (self.decode_mode >> digit) & 1 != 0 {
            CODE_B[usize::from(raw & 0x0F)] | (raw & 0x80)
        } else {
            raw
        }
    }

    /// Whether the matrix can emit light now (powered and not in shutdown, or in display test).
    fn active(&self) -> bool {
        self.powered && (self.display_test || !self.shutdown)
    }

    /// Recomputes `dot_a`, `lit_dots` and `frame` from registers and `i_seg_a`.
    fn refresh_frame(&mut self) {
        if !self.active() || self.i_seg_a <= 0.0 {
            self.dot_a = 0.0;
            self.lit_dots = 0;
            self.frame = [0; FRAME_BYTES];
            return;
        }
        let (duty, scan_digits) = if self.display_test {
            (31.0 / 32.0, 8.0)
        } else {
            (
                f64::from(2 * u16::from(self.intensity) + 1) / 32.0,
                f64::from(self.scan_limit) + 1.0,
            )
        };
        let dot_a = self.i_seg_a * duty / scan_digits;
        let level = ((dot_a / REF_DOT_A) * 255.0).round().clamp(1.0, 255.0) as u8;
        self.dot_a = dot_a;
        let mut lit = 0u8;
        for row in 0..8 {
            let bits = self.row_bits(row);
            for col in 0..8 {
                let on = (bits >> (7 - col)) & 1 != 0;
                self.frame[row * 8 + col] = if on {
                    lit += 1;
                    level
                } else {
                    0
                };
            }
        }
        self.lit_dots = lit;
    }

    fn latch(&mut self, cx: &mut Cx, at: Time) {
        let addr = ((self.shift >> 8) & 0x0F) as u8;
        let data = (self.shift & 0xFF) as u8;
        let before = self.frame;
        match addr {
            REG_NOOP => return,
            REG_DIGIT0..=REG_DIGIT7 => {
                self.digits[usize::from(addr - REG_DIGIT0)] = data;
            }
            REG_DECODE => self.decode_mode = data,
            REG_INTENSITY => self.intensity = data & 0x0F,
            REG_SCAN_LIMIT => self.scan_limit = data & 0x07,
            REG_SHUTDOWN => self.shutdown = (data & 0x01) == 0,
            REG_DISPLAY_TEST => self.display_test = (data & 0x01) != 0,
            _ => {
                cx.broke("command", at);
                return;
            }
        }
        self.refresh_frame();
        if self.frame != before {
            let mut hash = Sha256::new();
            hash.update(&self.frame);
            let mut short = [0u8; 8];
            short.copy_from_slice(&hash.finish()[..8]);
            cx.drew(Drawn {
                x0: 0,
                y0: 0,
                x1: WIDTH - 1,
                y1: HEIGHT - 1,
                bytes: FRAME_BYTES as u64,
                hash: short,
            });
        }
    }

    fn update_iset(&mut self, v_vcc: f64, v_iset: f64) {
        self.supply_v = v_vcc;
        let next_a = if v_vcc < MIN_VCC_V - 1e-6 {
            0.0
        } else {
            let i_iset = (v_iset - ISET_DROP_V) / ISET_RESISTANCE_OHM;
            if i_iset <= 1e-9 || v_vcc <= v_iset {
                0.0
            } else {
                let rset_ohm = (v_vcc - v_iset) / i_iset;
                let scale = ((v_vcc - ISET_DROP_V) / (5.0 - ISET_DROP_V)).max(0.0);
                table11_iseg(rset_ohm, self.vled_v) * scale
            }
        };
        // Quantize to 100 uA with 100 uA deadband so sub-millivolt rail ripple on a bypass
        // capacitor does not trigger 1 ps redraw stops in Native::follow_draw.
        let quantized = (next_a * 1e4).round() * 1e-4;
        if (quantized == 0.0) != (self.i_seg_a == 0.0) || (next_a - self.i_seg_a).abs() >= 1e-4 {
            self.i_seg_a = quantized;
        }
    }

    #[cfg(test)]
    pub(crate) fn digits(&self) -> [u8; 8] {
        self.digits
    }

    #[cfg(test)]
    pub(crate) fn dot_a(&self) -> f64 {
        self.dot_a
    }

    #[cfg(test)]
    pub(crate) fn iseg_a(&self) -> f64 {
        self.i_seg_a
    }
}

impl Device for Max7219 {
    const SHOWS: Shows = Shows::Screen {
        kind: "max7219",
        width: WIDTH,
        height: HEIGHT,
        format: "gray8",
    };
    const MIN_SUPPLY_V: Option<f64> = Some(MIN_VCC_V);
    const TRACKS_SUPPLY: bool = true;

    fn new(params: &Params<'_>) -> Result<Max7219, Error> {
        let vled_mv = params.int("vled_mv").unwrap_or(1_850);
        if vled_mv <= 0 {
            return Err(Error("vled_mv must be positive"));
        }
        let draw = match (
            params.int("shutdown_na"),
            params.int("logic_na"),
            params.int("holds_above_mv"),
        ) {
            (Some(shutdown_na), Some(logic_na), Some(holds_above_mv)) => {
                if shutdown_na < 0 || logic_na < 0 || holds_above_mv <= 0 {
                    return Err(Error("invalid draw params"));
                }
                Some(DrawSpec {
                    shutdown_a: shutdown_na as f64 * 1e-9,
                    logic_a: logic_na as f64 * 1e-9,
                    holds_above_v: holds_above_mv as f64 * 1e-3,
                })
            }
            (None, None, None) => None,
            _ => return Err(Error("draw params must all be set or all omitted")),
        };
        Ok(Max7219 {
            shift: 0,
            digits: [0; 8],
            decode_mode: 0,
            intensity: 0,
            scan_limit: 0,
            shutdown: true,
            display_test: false,
            dout: false,
            dout_due: None,
            prev: None,
            powered: false,
            supply_v: 0.0,
            i_seg_a: 0.0,
            dot_a: 0.0,
            lit_dots: 0,
            vled_v: vled_mv as f64 * 1e-3,
            draw,
            frame: [0; FRAME_BYTES],
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let (due, next) = self.dout_due.filter(|&(due, _)| due <= limit)?;
        self.dout = next;
        self.dout_due = None;
        Some(Stop {
            at: due,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        let mut drives = [Drive::Off; OUTPUTS];
        drives[OUTPUT_DOUT] = if self.dout { Drive::High } else { Drive::Low };
        drives[OUTPUT_ISET] = Drive::Low;
        Drives::new(drives)
    }

    fn draw(&self) -> Draw {
        let Some(spec) = self.draw else {
            return Draw::NONE;
        };
        if !self.powered {
            return Draw::NONE;
        }
        let amps = if self.active() {
            spec.logic_a + f64::from(self.lit_dots) * self.dot_a
        } else {
            spec.shutdown_a
        };
        Draw {
            amps,
            holds_above_v: spec.holds_above_v,
        }
    }

    fn input(&mut self, cx: &mut Cx, at: Time, levels: Levels) {
        if !levels.high(WATCH_SUPPLY) {
            self.powered = false;
            self.reset_registers();
            self.dout_due = if self.dout {
                Some((Self::later(at, 1), false))
            } else {
                None
            };
            self.refresh_frame();
            return;
        }
        self.powered = true;
        let now = [
            levels.high(WATCH_DIN),
            levels.high(WATCH_CLK),
            levels.high(WATCH_LOAD),
        ];
        let Some(prev) = self.prev else {
            self.prev = Some(now);
            self.refresh_frame();
            return;
        };
        self.prev = Some(now);
        let rise = |watch: usize| !prev[watch] && now[watch];
        let fall = |watch: usize| prev[watch] && !now[watch];

        // Datasheet p. 6: DIN shifts on every CLK rising edge regardless of LOAD; if CLK and LOAD
        // rise together, the shifted bit is included in the latched 16-bit word.
        if rise(WATCH_CLK) {
            self.shift = (self.shift << 1) | u16::from(now[WATCH_DIN]);
        }
        if fall(WATCH_CLK) {
            let next = (self.shift >> 15) & 1 != 0;
            if next != self.dout || self.dout_due.is_some() {
                self.dout_due = Some((Self::later(at, T_DO_PS), next));
            }
        }
        if rise(WATCH_LOAD) {
            self.latch(cx, at);
        }
    }

    fn sample(&mut self, _cx: &mut Cx, _at: Time, volts: &[f64]) {
        let v_vcc = volts.get(SAMPLE_VCC).copied().unwrap_or(0.0);
        let v_iset = volts.get(SAMPLE_ISET).copied().unwrap_or(0.0);
        let was_powered = self.powered;
        self.powered = v_vcc >= MIN_VCC_V - 1e-6;
        if !self.powered {
            if was_powered {
                self.reset_registers();
            }
            self.supply_v = v_vcc;
            self.i_seg_a = 0.0;
        } else {
            self.update_iset(v_vcc, v_iset);
        }
        self.refresh_frame();
    }

    fn view(&self) -> View<'_> {
        let iseg_10ua = (self.i_seg_a * 1e5).round().clamp(0.0, f64::from(u16::MAX)) as u16;
        let dot_ua = (self.dot_a * 1e6).round().clamp(0.0, f64::from(u16::MAX)) as u16;
        View::Screen {
            frame: &self.frame,
            panel: Panel {
                on: self.active(),
                inverted: false,
                reset: !self.powered,
                asleep: self.shutdown && !self.display_test,
                settings: Settings::new(
                    &SETTINGS,
                    [
                        u16::from(self.decode_mode),
                        u16::from(self.intensity),
                        u16::from(self.scan_limit),
                        u16::from(self.display_test),
                        iseg_10ua,
                        dot_ua,
                    ],
                ),
                registers: 0,
                backlight: None,
            },
        }
    }
}
