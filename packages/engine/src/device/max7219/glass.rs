//! What a `max7219`'s 8×8 LED matrix glass shows: 64 red LED dots whose brightness follows the
//! scan limit, intensity duty cycle, and `ISET` segment current.

use crate::display::Panel;

use super::model::{FRAME_BYTES, HEIGHT, WIDTH};

/// Red, green, blue and alpha, a byte each.
const RGBA: usize = 4;
/// An unlit white-diffused LED dot on a dark faceplate: stays below `display::lit`'s 128 ceiling
/// so `art()` marks it `.` while remaining visible as an unlit lens in the 3D viewer.
const OFF: [u8; RGBA] = [36, 34, 34, 255];

/// What the 8×8 matrix glass shows of `frame` (`gray8`, 64 bytes row-major) under `panel`.
///
/// Every lit dot (`frame[k] > 0`) renders with red >= 130 so `display::lit` always reads it lit
/// even at minimum 1/32 intensity, while scaling smoothly up to a vivid glowing red at full
/// brightness.
pub fn glass(panel: &Panel, frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame.len(), FRAME_BYTES, "a max7219 frame is 64 bytes");
    let (width, height) = (usize::from(WIDTH), usize::from(HEIGHT));
    let mut pixels = OFF.repeat(width * height);
    let active = panel.flags.display_on && !panel.flags.reset && !panel.flags.sleep;
    if !active {
        return pixels;
    }
    for (k, &level) in frame.iter().enumerate() {
        if level == 0 {
            continue;
        }
        let b = f64::from(level) / 255.0;
        let r = (130.0 + 125.0 * b.sqrt()).round().clamp(130.0, 255.0) as u8;
        let g = (52.0 * b * b).round().clamp(0.0, 255.0) as u8;
        let bl = (28.0 * b * b).round().clamp(0.0, 255.0) as u8;
        pixels[k * RGBA..][..RGBA].copy_from_slice(&[r, g, bl, 255]);
    }
    pixels
}

/// Per-dot average forward current in amperes (`8 × 8` rows, top to bottom, left to right).
/// Returns `0.0` for unlit dots and `dot_ua * 1e-6` for lit dots.
pub fn dot_brightness(panel: &Panel, frame: &[u8]) -> Vec<Vec<f64>> {
    assert_eq!(frame.len(), FRAME_BYTES, "a max7219 frame is 64 bytes");
    let active = panel.flags.display_on && !panel.flags.reset && !panel.flags.sleep;
    let dot_a = if active {
        f64::from(panel.settings.get("dot_ua").unwrap_or(0)) * 1e-6
    } else {
        0.0
    };
    let width = usize::from(WIDTH);
    let height = usize::from(HEIGHT);
    (0..height)
        .map(|row| {
            (0..width)
                .map(|col| {
                    if active && frame[row * width + col] > 0 {
                        dot_a
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{Flags, art, lit_count};
    use orbe_model::Settings;

    use super::super::model::SETTINGS;

    fn panel(on: bool, dot_ua: u16) -> Panel {
        Panel {
            kind: "max7219",
            width: WIDTH,
            height: HEIGHT,
            format: "gray8",
            flags: Flags {
                display_on: on,
                inverted: false,
                reset: false,
                sleep: !on,
            },
            settings: Settings::new(&SETTINGS, [0, 7, 7, 0, 4000, dot_ua]),
            registers: 0,
            brightness: None,
        }
    }

    #[test]
    fn even_minimum_intensity_reads_lit_and_scales_brighter_with_level() {
        let mut dim = [0u8; FRAME_BYTES];
        dim[0] = 1;
        let mut bright = [0u8; FRAME_BYTES];
        bright[0] = 255;
        let dim_px = glass(&panel(true, 156), &dim);
        let bright_px = glass(&panel(true, 4844), &bright);
        assert_eq!(lit_count(&dim_px), 1);
        assert_eq!(lit_count(&bright_px), 1);
        assert!(bright_px[0] > dim_px[0]);
        assert!(art(WIDTH, &dim_px).starts_with("#.......\n"));
    }

    #[test]
    fn shutdown_blanks_every_dot() {
        let full = [255u8; FRAME_BYTES];
        let px = glass(&panel(false, 4844), &full);
        assert_eq!(lit_count(&px), 0);
        let grid = dot_brightness(&panel(false, 4844), &full);
        assert!(grid.iter().flatten().all(|&a| a == 0.0));
    }
}
