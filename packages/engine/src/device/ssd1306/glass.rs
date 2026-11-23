//! What an `ssd1306`'s glass shows: its GDDRAM as lit and dark pixels on a 128 by 64 panel.
//!
//! The panel is wired for the configuration the module's makers send, and that Adafruit SSD1306
//! and U8g2 send: segment remap `A1h`, COM scan `C8h`, alternative COM pins `DAh 12h`. With
//! those, column address 0 is the left column and page 0 the top rows, with the header at the
//! top. So glass column `x` shows segment `127 - x`, and with the scan remapped glass row `y` is
//! row `y` of the scan.

use crate::display::Panel;

use super::model::{FRAME_BYTES, HEIGHT, WIDTH};

/// Red, green, blue and alpha, a byte each.
const RGBA: usize = 4;
/// A lit pixel, white, and a dark one, black. Both opaque.
const LIT: [u8; RGBA] = [255, 255, 255, 255];
const DARK: [u8; RGBA] = [0, 0, 0, 255];

/// What the glass shows of `frame`, the GDDRAM as `traces/display-<part>.mono-pages` holds it,
/// under `panel`: `WIDTH` by `HEIGHT` pixels of red, green, blue and alpha, rows top to bottom and
/// each row left to right, every one opaque white or black.
///
/// - With the display off, `RES#` low, or the charge pump off, every pixel is dark. The module
///   has no supply for the panel's high voltage besides the charge pump, and the pump is off
///   while the controller's `VDD` is below its logic minimum, so the glass is dark then too.
/// - A row the scan does not reach, past the multiplex ratio, is dark.
/// - Otherwise a pixel is lit when its GDDRAM bit is set, or for every pixel with entire display
///   on, `A5h`, and inverse display, `A7h`, flips it.
///
/// Contrast dims nothing here. Its value stays in the panel's settings for a viewer to apply.
///
/// Panics when `frame` is not 1024 bytes or `panel` lacks a setting an `ssd1306` carries.
pub fn glass(panel: &Panel, frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame.len(), FRAME_BYTES, "an ssd1306 GDDRAM is 1024 bytes");
    let setting = |name: &str| {
        panel
            .settings
            .get(name)
            .unwrap_or_else(|| panic!("an ssd1306 carries `{name}`")) as u8
    };
    let flags = panel.flags;
    let lit =
        flags.display_on && !flags.sleep && !flags.reset && setting("charge_pump") & 0x04 != 0;
    let (width, height) = (usize::from(WIDTH), usize::from(HEIGHT));
    let mut pixels = DARK.repeat(width * height);
    if !lit {
        return pixels;
    }
    let scan = Scan {
        start_line: setting("start_line"),
        offset: setting("offset"),
        multiplex: setting("multiplex"),
        remapped: setting("com_scan") != 0,
        com_pins: setting("com_pins"),
    };
    let entire = setting("entire_on") != 0;
    for y in 0..height {
        let Some(row) = scan.row(y as u8) else {
            continue;
        };
        let (page, bit) = (usize::from(row / 8), row % 8);
        for x in 0..width {
            let segment = width - 1 - x;
            let set = entire || frame[page * width + segment] >> bit & 1 != 0;
            if set != flags.inverted {
                pixels[(y * width + x) * RGBA..][..RGBA].copy_from_slice(&LIT);
            }
        }
    }
    pixels
}

/// The settings that pick the GDDRAM row each glass row shows.
struct Scan {
    start_line: u8,
    offset: u8,
    multiplex: u8,
    remapped: bool,
    com_pins: u8,
}

impl Scan {
    /// The GDDRAM row glass row `y` shows, or `None` when the scan does not reach it.
    ///
    /// The panel's row `y` sits on the COM line that row `y` of an alternative, remapped scan
    /// drives. A normal scan, `C0h`, runs the other way, so the picture turns upside down. A
    /// sequential COM pin configuration on this panel puts the scan's row `2y` on the top half's
    /// row `y` and row `2y - 63` on the bottom half's, so the picture shows every other row
    /// twice over. Left and right COM remap, bit 5, swaps the scan's halves. The display offset
    /// moves the picture up by its rows, a row of the scan past the multiplex ratio stays dark,
    /// and the display start line is the GDDRAM row the scan's first row shows (datasheet 10.1.6,
    /// 10.1.12, 10.1.14, 10.1.15 and 10.1.18).
    fn row(&self, y: u8) -> Option<u8> {
        let mut line = if self.remapped { y } else { 63 - y };
        if self.com_pins & 0x10 == 0 {
            line = if line < 32 { 2 * line } else { 2 * line - 63 };
        }
        if self.com_pins & 0x20 != 0 {
            line = (line + 32) % 64;
        }
        let counter = (line + self.offset) % 64;
        (counter <= self.multiplex).then_some((counter + self.start_line) % 64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::Flags;
    use orbe_model::Settings;

    use super::super::model::SETTINGS;

    const W: usize = WIDTH as usize;

    /// The settings Adafruit SSD1306 and U8g2 leave a 128 by 64 panel in.
    fn settings(change: &[(&str, u16)]) -> Settings {
        let mut values = [0xcf, 0, 0, 0, 0, 63, 1, 1, 0x12, 0x14];
        for &(name, value) in change {
            let k = SETTINGS.iter().position(|&s| s == name).unwrap();
            values[k] = value;
        }
        Settings::new(&SETTINGS, values)
    }

    fn panel(flags: Flags, change: &[(&str, u16)]) -> Panel {
        Panel {
            kind: "ssd1306",
            width: WIDTH,
            height: HEIGHT,
            format: "mono-pages",
            flags,
            settings: settings(change),
            registers: 0,
            brightness: None,
        }
    }

    fn on() -> Flags {
        Flags {
            display_on: true,
            inverted: false,
            reset: false,
            sleep: false,
        }
    }

    /// The GDDRAM after a write at column address `column`, row `row`, with segment remap on.
    fn dot(column: usize, row: usize) -> Vec<u8> {
        let mut frame = vec![0; FRAME_BYTES];
        frame[row / 8 * W + (W - 1 - column)] = 1 << (row % 8);
        frame
    }

    fn lit(pixels: &[u8]) -> Vec<(usize, usize)> {
        (0..W * usize::from(HEIGHT))
            .filter(|k| pixels[k * RGBA] == 255)
            .map(|k| (k % W, k / W))
            .collect()
    }

    #[test]
    fn a_dot_written_at_column_3_row_10_lights_that_pixel() {
        assert_eq!(lit(&glass(&panel(on(), &[]), &dot(3, 10))), [(3, 10)]);
    }

    #[test]
    fn display_off_reset_and_no_charge_pump_each_show_every_pixel_dark() {
        let full = vec![0xff; FRAME_BYTES];
        for (flags, change) in [
            (
                Flags {
                    display_on: false,
                    sleep: true,
                    ..on()
                },
                &[][..],
            ),
            (
                Flags {
                    reset: true,
                    ..on()
                },
                &[],
            ),
            (on(), &[("charge_pump", 0x10)]),
        ] {
            assert!(
                lit(&glass(&panel(flags, change), &full)).is_empty(),
                "{flags:?}"
            );
        }
        assert_eq!(lit(&glass(&panel(on(), &[]), &full)).len(), 128 * 64);
    }

    #[test]
    fn inverse_and_entire_on_flip_and_fill() {
        let inverted = Flags {
            inverted: true,
            ..on()
        };
        assert_eq!(
            lit(&glass(&panel(inverted, &[]), &dot(3, 10))).len(),
            128 * 64 - 1
        );
        let all = glass(&panel(on(), &[("entire_on", 1)]), &vec![0; FRAME_BYTES]);
        assert_eq!(lit(&all).len(), 128 * 64);
    }

    #[test]
    fn a_normal_scan_turns_the_picture_upside_down() {
        let normal = panel(on(), &[("com_scan", 0)]);
        assert_eq!(lit(&glass(&normal, &dot(3, 10))), [(3, 53)]);
    }

    #[test]
    fn start_line_and_offset_move_the_picture_up() {
        assert_eq!(
            lit(&glass(&panel(on(), &[("start_line", 4)]), &dot(3, 10))),
            [(3, 6)]
        );
        assert_eq!(
            lit(&glass(&panel(on(), &[("offset", 4)]), &dot(3, 10))),
            [(3, 6)]
        );
    }

    #[test]
    fn a_row_past_the_multiplex_ratio_stays_dark() {
        let short = panel(on(), &[("multiplex", 31)]);
        assert_eq!(lit(&glass(&short, &dot(3, 10))), [(3, 10)]);
        assert!(lit(&glass(&short, &dot(3, 40))).is_empty());
    }

    #[test]
    fn a_sequential_com_configuration_shows_every_other_row() {
        let sequential = panel(on(), &[("com_pins", 0x02)]);
        assert_eq!(lit(&glass(&sequential, &dot(3, 20))), [(3, 10)]);
        assert_eq!(lit(&glass(&sequential, &dot(3, 21))), [(3, 42)]);
    }
}
