use crate::world::World;
use orbe_model::{
    Cs, Device, Drawn, Fields, Lanes, Miso, Panel, Sha256, Spi, SpiPhase, Time, View,
};

use super::Port;
use super::model::{
    CASET, COLMOD, DISPON, MADCTL, MV, MX, MY, OUTPUTS, RAMWR, RAMWRC, RASET, St77916, WATCH_RST,
    WIDTH, place,
};
use crate::active::native::{Hosted, Native};
use crate::active::{duty, spi};
use crate::diagnostic::ProtocolRule;
use crate::link::{Answers, LinkRow, Record, RecordKind, SpiNets};
use crate::netlist::NetId;

const OPCODE_PIXELS: u32 = 0x32;
const ROW_BYTES: usize = WIDTH as usize * 2;

/// A pixel write the panel closed, with its time.
#[derive(Clone, Debug, PartialEq)]
struct Closed {
    at: Time,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
    bytes: u64,
    hash: [u8; 8],
}

/// The panel behind its SPI engine and its PWM pin, with every data lane on one net, and the
/// pixel writes it closed so far.
struct Core {
    native: Native<St77916, Port>,
    closed: Vec<Closed>,
}

fn panel(native: &Native<St77916, Port>) -> Panel {
    let View::Screen { panel, .. } = native.view() else {
        panic!("a panel shows a screen");
    };
    panel
}

impl Core {
    fn new() -> Core {
        let bus = spi::Bus {
            data: lanes(),
            reset: Some(WATCH_RST),
        };
        Core {
            native: Native::new(
                St77916::new(&Fields::NONE).unwrap(),
                (spi::Engine::new(bus), duty::Pin),
                OUTPUTS,
            ),
            closed: Vec::new(),
        }
    }

    fn input(&mut self, at: Time, levels: &[bool]) {
        self.native.input(&World::default(), at, levels);
    }

    fn take(&mut self, at: Time, records: &[LinkRow]) {
        self.native
            .take(&World::default(), at, records, &mut Answers::default());
        let closed = &mut self.closed;
        self.native.drain_rows(&mut |at, drawn: Drawn| {
            closed.push(Closed {
                at,
                x0: drawn.x0,
                y0: drawn.y0,
                x1: drawn.x1,
                y1: drawn.y1,
                bytes: drawn.bytes,
                hash: drawn.hash,
            });
        });
    }

    fn panel(&self) -> Panel {
        panel(&self.native)
    }

    fn image(&self) -> Vec<u8> {
        let View::Screen { frame, .. } = self.native.view() else {
            panic!("a panel shows a screen");
        };
        frame.to_vec()
    }

    fn broke_rules(&self) -> impl Iterator<Item = (ProtocolRule, Time)> {
        self.native.broke().into_iter()
    }
}

fn lanes() -> [NetId; 4] {
    [NetId::GROUND; 4]
}

fn spi(phases: &[(u8, u32)], data: &[u8], data_lanes: Lanes, cs: Cs) -> LinkRow {
    LinkRow {
        sender: 0,
        record: Record {
            seq: 0,
            start: Time::ZERO,
            end: Time::ZERO,
            pin: "CS",
            kind: RecordKind::Spi {
                spi: Spi {
                    phases: phases
                        .iter()
                        .map(|&(bits, value)| SpiPhase {
                            bits,
                            lanes: Lanes::One,
                            value,
                        })
                        .collect(),
                    lanes: data_lanes,
                    data: data.into(),
                    clock_hz: 1,
                    cs,
                    miso: Miso::None,
                },
                nets: SpiNets {
                    cs: Some(NetId::GROUND),
                    sclk: Some(NetId::GROUND),
                    data: lanes().map(Some),
                },
            },
        },
    }
}

fn write(opcode: u32, command: u8, data: &[u8]) -> LinkRow {
    let lanes = if opcode == OPCODE_PIXELS {
        Lanes::Four
    } else {
        Lanes::One
    };
    spi(
        &[(8, opcode), (24, u32::from(command) << 8)],
        data,
        lanes,
        Cs::Released,
    )
}

fn pixel(image: &[u8], x: usize, y: usize) -> &[u8] {
    &image[y * ROW_BYTES + x * 2..][..2]
}

fn ready() -> Core {
    let mut core = Core::new();
    core.input(Time::ZERO, &[true]);
    core.take(Time::ZERO, &[write(0x02, COLMOD, &[0x55])]);
    core
}

/// A write over rows 0 to 20 lands in pages 0, 1 and 2 of 8 rows each. A copy over its twin
/// copies those three and no other.
#[test]
fn a_copy_after_a_write_over_three_bands_copies_three_bands() {
    let mut core = ready();
    let committed = core.native.clone();
    assert_eq!(committed.device().pages_to_copy(core.native.device()), 0);
    core.take(
        Time::from_ps(1),
        &[
            write(0x02, RASET, &[0, 0, 0, 20]),
            write(0x02, CASET, &[0, 0, 0, 0]),
            write(0x32, RAMWR, &[0xab; 42]),
        ],
    );
    assert_eq!(committed.device().pages_to_copy(core.native.device()), 3);
    let mut copy = committed.clone();
    copy.copy_from(&core.native);
    assert_eq!(copy.device().pages_to_copy(core.native.device()), 0);
    assert_eq!(panel(&copy), core.panel());
    let View::Screen { frame, .. } = copy.view() else {
        unreachable!()
    };
    assert_eq!(frame, core.image());
    assert_eq!(pixel(frame, 0, 20), [0xab, 0xab]);
}

#[test]
fn a_held_write_continues_in_the_next_data_only_transaction() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[
            write(0x02, CASET, &[0, 1, 0, 2]),
            write(0x02, RASET, &[0, 3, 0, 3]),
            spi(&[(8, 0x32), (24, 0x2c00)], &[1, 2], Lanes::Four, Cs::Held),
        ],
    );
    assert!(core.closed.is_empty(), "a held write is open");
    core.take(
        Time::from_ps(2),
        &[spi(&[], &[3, 4, 5, 6], Lanes::Four, Cs::Released)],
    );
    let mut hash = Sha256::new();
    hash.update(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(
        core.closed,
        [Closed {
            at: Time::from_ps(2),
            x0: 1,
            y0: 3,
            x1: 2,
            y1: 3,
            bytes: 6,
            hash: hash.finish()[..8].try_into().unwrap(),
        }]
    );
    let image = core.image();
    let at = |x: usize, y: usize| &image[y * ROW_BYTES + x * 2..][..2];
    assert_eq!((at(1, 3), at(2, 3)), (&[5, 6][..], &[3, 4][..]));
}

#[test]
fn a_command_word_sent_as_bytes_opens_the_command_the_next_bytes_continue() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[
            spi(&[], &[0x02, 0x00, MADCTL, 0x00], Lanes::One, Cs::Held),
            spi(&[], &[MX], Lanes::One, Cs::Released),
            spi(&[(8, 0x32)], &[0x00, RAMWR, 0x00], Lanes::One, Cs::Held),
            spi(&[], &[0x11, 0x22], Lanes::Four, Cs::Released),
        ],
    );
    assert_eq!(core.panel().settings.get("madctl").unwrap(), u16::from(MX));
    assert_eq!(pixel(&core.image(), 359, 0), &[0x11, 0x22]);
    assert_eq!(core.closed.len(), 1);
    assert_eq!(core.broke_rules().count(), 0);
    let mut split = ready();
    split.take(
        Time::from_ps(1),
        &[spi(
            &[],
            &[0x02, 0x00, COLMOD, 0x00, 0x66],
            Lanes::One,
            Cs::Released,
        )],
    );
    assert_eq!(split.panel().settings.get("colmod").unwrap(), 0x66);
    assert_eq!(split.broke_rules().count(), 0);
}

#[test]
fn a_command_word_a_release_cuts_short_or_phases_past_it_break_command() {
    let mut core = ready();
    core.take(
        Time::from_ps(5),
        &[spi(&[], &[0x02, 0x00], Lanes::One, Cs::Released)],
    );
    core.take(Time::from_ps(6), &[write(0x02, DISPON, &[])]);
    assert!(core.panel().on, "a release ends the cut word");
    assert_eq!(
        core.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(5))]
    );
    let mut past = ready();
    past.take(
        Time::from_ps(7),
        &[spi(
            &[(8, 0x02), (32, 0x0200_2900)],
            &[],
            Lanes::One,
            Cs::Released,
        )],
    );
    assert!(!past.panel().on);
    assert_eq!(
        past.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(7))]
    );
    let mut held = ready();
    held.take(
        Time::from_ps(8),
        &[
            spi(
                &[(8, 0x02), (24, u32::from(COLMOD) << 8)],
                &[],
                Lanes::One,
                Cs::Held,
            ),
            spi(
                &[(8, 0x02), (24, u32::from(DISPON) << 8)],
                &[],
                Lanes::One,
                Cs::Held,
            ),
            spi(&[], &[0x66], Lanes::One, Cs::Released),
        ],
    );
    assert!(!held.panel().on, "phases inside a command are dropped");
    assert_eq!(held.panel().settings.get("colmod").unwrap(), 0x66);
    assert_eq!(
        held.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(8))]
    );
}

#[test]
fn a_command_word_on_more_than_one_lane_breaks_command() {
    let quad_bytes = spi(&[], &[0x02, 0x00, DISPON, 0x00], Lanes::Four, Cs::Released);
    let mut quad_phase = spi(
        &[(8, 0x02), (24, u32::from(DISPON) << 8)],
        &[],
        Lanes::One,
        Cs::Released,
    );
    if let RecordKind::Spi { spi, .. } = &mut quad_phase.record.kind {
        spi.phases[1].lanes = Lanes::Four;
    }
    for (k, row) in [quad_bytes, quad_phase].into_iter().enumerate() {
        let mut core = ready();
        core.take(Time::from_ps(9), &[row]);
        assert!(!core.panel().on, "case {k}");
        assert_eq!(
            core.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(9))],
            "case {k}"
        );
    }
    let mut one = ready();
    one.take(
        Time::from_ps(9),
        &[spi(
            &[],
            &[0x02, 0x00, DISPON, 0x00],
            Lanes::One,
            Cs::Released,
        )],
    );
    assert!(one.panel().on);
}

#[test]
fn madctl_mirrors_then_exchanges() {
    assert_eq!(place(0, (3, 7)), (3, 7));
    assert_eq!(place(MX, (3, 7)), (356, 7));
    assert_eq!(place(MY, (3, 7)), (3, 352));
    assert_eq!(place(MV, (3, 7)), (7, 3));
    assert_eq!(place(MX | MV, (3, 7)), (7, 356));
}

#[test]
fn madctl_with_mirror_and_swap_lands_the_pixel_after_the_exchange() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[
            write(0x02, CASET, &[0, 3, 0, 3]),
            write(0x02, RASET, &[0, 7, 0, 7]),
            write(0x02, MADCTL, &[MX | MV]),
            write(0x32, RAMWR, &[0xbe, 0xef]),
        ],
    );
    let image = core.image();
    assert_eq!(pixel(&image, 7, 356), &[0xbe, 0xef]);
    assert_eq!(pixel(&image, 356, 7), &[0, 0]);
    assert_eq!(pixel(&image, 352, 3), &[0, 0]);
}

#[test]
fn a_pixel_opcode_on_a_register_command_runs_nothing() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x32), (24, u32::from(CASET) << 8)],
            &[0, 1, 0, 2],
            Lanes::Four,
            Cs::Released,
        )],
    );
    core.take(Time::from_ps(2), &[write(0x32, RAMWR, &[9, 8])]);
    let image = core.image();
    assert_eq!(pixel(&image, 0, 0), &[9, 8]);
    assert_eq!(pixel(&image, 1, 0), &[0, 0]);
    assert_eq!(
        core.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(1))]
    );
}

#[test]
fn a_register_opcode_on_a_pixel_command_runs_nothing() {
    for command in [RAMWR, RAMWRC] {
        let mut core = ready();
        core.take(
            Time::from_ps(4),
            &[spi(
                &[(8, 0x02), (24, u32::from(command) << 8)],
                &[9, 8],
                Lanes::One,
                Cs::Released,
            )],
        );
        assert!(core.closed.is_empty(), "command {command:#x}");
        assert_eq!(pixel(&core.image(), 0, 0), &[0, 0]);
        assert_eq!(
            core.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(4))]
        );
    }
}

#[test]
fn data_lanes_that_do_not_match_the_opcode_run_nothing() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x02), (24, u32::from(COLMOD) << 8)],
            &[0x66],
            Lanes::Four,
            Cs::Released,
        )],
    );
    assert_eq!(core.panel().settings.get("colmod").unwrap(), 0x55);
    let mut pixels = ready();
    pixels.take(Time::from_ps(2), &[write(0x32, RAMWR, &[9, 9])]);
    pixels.take(
        Time::from_ps(3),
        &[spi(
            &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
            &[1, 2],
            Lanes::One,
            Cs::Released,
        )],
    );
    pixels.take(Time::from_ps(4), &[write(0x32, RAMWRC, &[3, 4])]);
    assert_eq!(pixels.closed.len(), 2);
    let image = pixels.image();
    assert_eq!(pixel(&image, 0, 0), &[9, 9]);
    assert_eq!(
        pixel(&image, 1, 0),
        &[3, 4],
        "the dropped RAMWR left the cursor"
    );
    assert_eq!(
        pixels.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(3))]
    );
}

#[test]
fn a_nonzero_middle_byte_of_the_command_word_is_ignored() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[
            spi(
                &[(8, 0x02), (24, 0x00ab_2a00)],
                &[0, 5, 0, 6],
                Lanes::One,
                Cs::Released,
            ),
            write(0x32, RAMWR, &[3, 4]),
        ],
    );
    assert_eq!(pixel(&core.image(), 5, 0), &[3, 4]);
    assert_eq!(pixel(&core.image(), 0, 0), &[0, 0]);
    assert_eq!(core.panel().registers, 0);
    assert_eq!(core.broke_rules().count(), 0);
}

#[test]
fn a_write_wraps_to_the_top_left_of_a_three_row_window() {
    let mut core = ready();
    let mut data = Vec::new();
    for i in 0..7u8 {
        data.extend([i, 0x10 + i]);
    }
    core.take(
        Time::from_ps(1),
        &[
            write(0x02, CASET, &[0, 1, 0, 2]),
            write(0x02, RASET, &[0, 5, 0, 7]),
            write(0x32, RAMWR, &data),
        ],
    );
    let image = core.image();
    assert_eq!(pixel(&image, 1, 5), &[6, 0x16]);
    assert_eq!(pixel(&image, 2, 5), &[1, 0x11]);
    assert_eq!(pixel(&image, 2, 7), &[5, 0x15]);
    assert_eq!(core.closed[0].bytes, 14);
    assert_eq!((core.closed[0].x0, core.closed[0].y0), (1, 5));
    assert_eq!((core.closed[0].x1, core.closed[0].y1), (2, 7));
}

#[test]
fn ramwrc_continues_from_where_ramwr_stopped() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[
            write(0x02, CASET, &[0, 1, 0, 2]),
            write(0x02, RASET, &[0, 4, 0, 4]),
            write(0x32, RAMWR, &[1, 2]),
        ],
    );
    core.take(Time::from_ps(2), &[write(0x32, RAMWRC, &[3, 4])]);
    let image = core.image();
    assert_eq!(pixel(&image, 1, 4), &[1, 2]);
    assert_eq!(pixel(&image, 2, 4), &[3, 4]);
    assert_eq!(core.closed.len(), 2);
    assert_eq!(core.closed[0].bytes, 2);
    assert_eq!(core.closed[1].bytes, 2);
    assert_eq!((core.closed[1].x0, core.closed[1].y0), (1, 4));
}

#[test]
fn a_pixel_split_on_an_odd_byte_joins_across_two_records() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
            &[0x11, 0x22, 0x33],
            Lanes::Four,
            Cs::Held,
        )],
    );
    assert!(core.closed.is_empty());
    core.take(
        Time::from_ps(2),
        &[spi(&[], &[0x44, 0x55, 0x66], Lanes::Four, Cs::Released)],
    );
    let image = core.image();
    assert_eq!(pixel(&image, 0, 0), &[0x11, 0x22]);
    assert_eq!(pixel(&image, 1, 0), &[0x33, 0x44]);
    assert_eq!(pixel(&image, 2, 0), &[0x55, 0x66]);
    assert_eq!(core.closed[0].bytes, 6);
}

#[test]
fn a_register_the_model_stores_counts_once_however_often_it_is_written() {
    let mut core = ready();
    let before = core.panel();
    for _ in 0..300 {
        core.take(Time::from_ps(1), &[write(0x02, 0xf0, &[1, 2, 3])]);
    }
    assert_eq!(core.panel().registers, 1);
    assert_eq!(
        core.panel().settings.get("madctl").unwrap(),
        before.settings.get("madctl").unwrap()
    );
    assert_eq!(
        core.panel().settings.get("colmod").unwrap(),
        before.settings.get("colmod").unwrap()
    );
    assert_eq!(
        (core.panel().on, core.panel().asleep, core.panel().inverted),
        (before.on, before.asleep, before.inverted)
    );
    let committed = core.native.clone();
    core.take(Time::from_ps(2), &[write(0x02, 0xf0, &[9])]);
    assert_eq!(core.panel().registers, 1);
    core.take(Time::from_ps(3), &[write(0x02, 0xf1, &[4, 5])]);
    assert_eq!(core.panel().registers, 2);
    assert_eq!(panel(&committed).registers, 1, "a copy keeps its own count");
    core.take(Time::from_ps(4), &[write(0x02, 0x01, &[])]);
    assert_eq!(core.panel().registers, 0, "SWRESET clears what it stored");
}

#[test]
fn a_ramwr_sent_alone_on_one_lane_draws_the_four_lane_data() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
            &[],
            Lanes::One,
            Cs::Held,
        )],
    );
    assert!(core.closed.is_empty(), "a command with no bytes stays open");
    assert_eq!(core.broke_rules().count(), 0);
    core.take(
        Time::from_ps(2),
        &[spi(&[], &[0x11, 0x22], Lanes::Four, Cs::Released)],
    );
    assert_eq!(pixel(&core.image(), 0, 0), &[0x11, 0x22]);
    assert_eq!(core.closed.len(), 1);
    assert_eq!(core.closed[0].bytes, 2);
    assert_eq!(core.broke_rules().count(), 0);
}

#[test]
fn a_dispon_with_no_bytes_labelled_four_lanes_runs() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x02), (24, u32::from(DISPON) << 8)],
            &[],
            Lanes::Four,
            Cs::Released,
        )],
    );
    assert!(core.panel().on);
    assert_eq!(core.broke_rules().count(), 0);
    let mut crossed = ready();
    crossed.take(
        Time::from_ps(2),
        &[spi(
            &[(8, 0x32), (24, u32::from(DISPON) << 8)],
            &[],
            Lanes::One,
            Cs::Released,
        )],
    );
    assert!(!crossed.panel().on);
    assert_eq!(
        crossed.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(2))]
    );
}

#[test]
fn a_continuation_on_the_wrong_lanes_drops_its_bytes() {
    let mut core = ready();
    core.take(
        Time::from_ps(1),
        &[spi(
            &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
            &[0x11, 0x22],
            Lanes::Four,
            Cs::Held,
        )],
    );
    core.take(
        Time::from_ps(2),
        &[spi(&[], &[0x33, 0x44], Lanes::One, Cs::Held)],
    );
    core.take(
        Time::from_ps(3),
        &[spi(&[], &[0x55, 0x66], Lanes::Four, Cs::Released)],
    );
    let image = core.image();
    assert_eq!(pixel(&image, 0, 0), &[0x11, 0x22]);
    assert_eq!(pixel(&image, 1, 0), &[0x55, 0x66]);
    assert_eq!(pixel(&image, 2, 0), &[0, 0]);
    assert_eq!(core.closed.len(), 1);
    assert_eq!(core.closed[0].bytes, 4);
    assert_eq!(
        core.broke_rules().collect::<Vec<_>>(),
        [(ProtocolRule::Command, Time::from_ps(2))]
    );
}
