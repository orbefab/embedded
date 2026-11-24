//! What a blue 16×2 HD44780 LCD's glass shows: both visible 16-character lines and the rendered
//! 320×68 RGBA image using the A00 5×8 ROM font and custom CGRAM glyphs.
//!
//! On the common white-on-blue negative transmissive 1602 LCD, characters are only visible when
//! both the display and the backlight are on. When unpowered, the display is off, or the
//! backlight is off, [`visible_lines`] returns two 16-space blank lines and [`glass`] draws dark
//! unlit glass.

use crate::display::Panel;

use super::font::{char_for, glyph};
use super::model::{DDRAM_BYTES, FRAME_BYTES, HEIGHT, WIDTH};

const COLS: usize = 16;
const ROWS: usize = 2;
const RGBA: usize = 4;

// Each LCD pixel is drawn as a 2×2 block on a 3 px pitch, giving a 14×23 px 5×8 character cell.
// 16 cells with 4 px gaps = 284 px wide (18 px left/right margin in 320 px); 2 rows with a 6 px
// gap = 52 px tall (8 px top/bottom margin in 68 px).
const DOT_SIZE: usize = 2;
const DOT_STEP: usize = 3;
const CELL_W: usize = 5 * DOT_STEP - 1; // 14
const CELL_H: usize = 8 * DOT_STEP - 1; // 23
const COL_GAP: usize = 4;
const ROW_GAP: usize = 6;
const MARGIN_X: usize = (WIDTH as usize - (COLS * CELL_W + (COLS - 1) * COL_GAP)) / 2; // 18
const MARGIN_Y: usize = (HEIGHT as usize - (ROWS * CELL_H + (ROWS - 1) * ROW_GAP)) / 2; // 8

const BACKLIT_BG: [u8; RGBA] = [16, 84, 180, 255];
const BACKLIT_OFF_DOT: [u8; RGBA] = [24, 98, 198, 255];
const BACKLIT_ON_DOT: [u8; RGBA] = [255, 255, 255, 255];
const DARK_BG: [u8; RGBA] = [10, 18, 32, 255];
const DARK_OFF_DOT: [u8; RGBA] = [13, 22, 38, 255];

fn setting(panel: &Panel, name: &str) -> u8 {
    panel
        .settings
        .get(name)
        .unwrap_or_else(|| panic!("an hd44780_i2c carries `{name}`")) as u8
}

/// Whether the backlight is lit (powered, jumper fitted, and PCF8574 P3 high).
pub fn backlight_on(panel: &Panel) -> bool {
    setting(panel, "powered") != 0 && setting(panel, "backlight") != 0
}

/// Whether characters are visible on the blue transmissive glass: powered, display on, and
/// backlight on.
pub fn text_visible(panel: &Panel) -> bool {
    panel.flags.display_on && !panel.flags.reset && !panel.flags.sleep && backlight_on(panel)
}

/// The two visible 16-character lines on the glass after display shift.
///
/// Both lines are 16 spaces when the display is unpowered, off, or its backlight is off.
pub fn visible_lines(panel: &Panel, frame: &[u8]) -> [String; 2] {
    assert_eq!(
        frame.len(),
        FRAME_BYTES,
        "an hd44780_i2c frame is 144 bytes (80 DDRAM + 64 CGRAM)"
    );
    if !text_visible(panel) {
        return [" ".repeat(COLS), " ".repeat(COLS)];
    }
    let two_line = setting(panel, "two_line") != 0;
    let shift = usize::from(setting(panel, "shift"));
    let ddram = &frame[..DDRAM_BYTES];
    let mut lines = [String::with_capacity(COLS), String::with_capacity(COLS)];
    for (row, line) in lines.iter_mut().enumerate() {
        if row == 1 && !two_line {
            line.push_str(&" ".repeat(COLS));
            continue;
        }
        for col in 0..COLS {
            let idx = visible_ddram_index(two_line, shift, row, col);
            line.push(char_for(ddram[idx]));
        }
    }
    lines
}

/// What the 16×2 blue LCD glass shows as `WIDTH × HEIGHT` RGBA pixels.
pub fn glass(panel: &Panel, frame: &[u8]) -> Vec<u8> {
    assert_eq!(
        frame.len(),
        FRAME_BYTES,
        "an hd44780_i2c frame is 144 bytes (80 DDRAM + 64 CGRAM)"
    );
    let width = usize::from(WIDTH);
    let height = usize::from(HEIGHT);
    let lit_bl = backlight_on(panel);
    let bg = if lit_bl { BACKLIT_BG } else { DARK_BG };
    let off_dot = if lit_bl {
        BACKLIT_OFF_DOT
    } else {
        DARK_OFF_DOT
    };
    let mut pixels = bg.repeat(width * height);

    let show_chars = text_visible(panel);
    let two_line = setting(panel, "two_line") != 0;
    let shift = usize::from(setting(panel, "shift"));
    let cursor = setting(panel, "cursor") != 0;
    let blink = setting(panel, "blink") != 0;
    let ac = setting(panel, "ac");
    let in_cgram = setting(panel, "in_cgram") != 0;
    let ddram = &frame[..DDRAM_BYTES];
    let cgram = &frame[DDRAM_BYTES..];

    for row in 0..ROWS {
        let cell_y = MARGIN_Y + row * (CELL_H + ROW_GAP);
        for col in 0..COLS {
            let cell_x = MARGIN_X + col * (CELL_W + COL_GAP);
            let active_row = row == 0 || two_line;
            let ddram_addr = visible_ddram_addr(two_line, shift, row, col);
            let at_cursor = show_chars && active_row && !in_cgram && ac == ddram_addr;
            let bitmap = if show_chars && active_row {
                let idx = visible_ddram_index(two_line, shift, row, col);
                glyph(ddram[idx], cgram)
            } else {
                [0u8; 8]
            };

            for (dot_row, &bits) in bitmap.iter().enumerate() {
                for dot_col in 0..5 {
                    let pixel_on = show_chars
                        && active_row
                        && ((at_cursor && (blink || (cursor && dot_row == 7)))
                            || ((bits >> (4 - dot_col)) & 1 != 0));
                    let color = if pixel_on { BACKLIT_ON_DOT } else { off_dot };
                    let px0 = cell_x + dot_col * DOT_STEP;
                    let py0 = cell_y + dot_row * DOT_STEP;
                    for dy in 0..DOT_SIZE {
                        for dx in 0..DOT_SIZE {
                            let off = ((py0 + dy) * width + (px0 + dx)) * RGBA;
                            pixels[off..off + RGBA].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }
    pixels
}

fn visible_ddram_index(two_line: bool, shift: usize, row: usize, col: usize) -> usize {
    if two_line {
        row * 40 + (shift + col) % 40
    } else {
        (shift + col) % DDRAM_BYTES
    }
}

fn visible_ddram_addr(two_line: bool, shift: usize, row: usize, col: usize) -> u8 {
    if two_line {
        let offset = ((shift + col) % 40) as u8;
        if row == 0 { offset } else { 0x40 + offset }
    } else {
        ((shift + col) % DDRAM_BYTES) as u8
    }
}
