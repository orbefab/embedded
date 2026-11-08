//! What an `st77916`'s glass shows: its frame memory as visible pixels on a round panel.

use orbe_engine_core::display::Flags;

use super::{HEIGHT, WIDTH};

/// Red, green, blue and alpha, a byte each.
const RGBA: usize = 4;

/// What the glass shows of `frame`, the frame memory as `traces/display-<part>.rgb565` holds it,
/// under `flags`: `WIDTH` by `HEIGHT` pixels of red, green, blue and alpha, a byte each, rows top
/// to bottom and each row left to right.
///
/// - A pixel off the round glass, as [`on_glass`] draws it, is `[0, 0, 0, 0]`.
/// - With the display flag clear, the sleep flag set, or `RST` low, every pixel on the glass is
///   opaque black.
/// - Otherwise each pixel is its two bytes as big-endian RGB565, each channel widened to 8 bits
///   by bit replication. This panel shows the frame memory as written with the inverted flag
///   set, and each channel inverted with it clear.
///
/// The backlight dims nothing here. Its duty stays in the panel state for a viewer to apply.
///
/// Panics when `frame` is not `WIDTH * HEIGHT * 2` bytes.
pub fn glass(flags: Flags, frame: &[u8]) -> Vec<u8> {
    let (width, height) = (usize::from(WIDTH), usize::from(HEIGHT));
    assert_eq!(
        frame.len(),
        width * height * 2,
        "an st77916 frame memory is two bytes a pixel"
    );
    let lit = flags.display_on && !flags.sleep && !flags.reset;
    // With the vendor start-up and INVON, a board showed red, green, blue, white and black bands
    // written high byte first as those colours. So INVON shows the frame memory as written.
    let invert = if flags.inverted { 0 } else { 0xff };
    let mut pixels = vec![0; width * height * RGBA];
    let visible = pixels
        .as_chunks_mut::<RGBA>()
        .0
        .iter_mut()
        .zip(frame.as_chunks::<2>().0);
    for (k, (out, pixel)) in visible.enumerate() {
        if !on_glass(k % width, k / width) {
            continue;
        }
        if lit {
            let rgb = widen(u16::from_be_bytes(*pixel));
            out[..3].copy_from_slice(&rgb.map(|channel| channel ^ invert));
        }
        out[3] = 0xff;
    }
    pixels
}

/// Whether pixel `(x, y)` is on the glass: its centre lies inside the circle of diameter `WIDTH`
/// about the panel's centre, the corner pixels 179 and 180 share on both axes. Doubled, a pixel's
/// centre is `(2x + 1, 2y + 1)` and the circle's is `(360, 360)`, so the test is exact in
/// integers. No pixel's centre lies on the circle, because two odd squares never sum to 360².
fn on_glass(x: usize, y: usize) -> bool {
    let diameter = i32::from(WIDTH);
    let offset = |k: usize| 2 * i32::try_from(k).expect("a column fits") + 1 - diameter;
    let (dx, dy) = (offset(x), offset(y));
    dx * dx + dy * dy <= diameter * diameter
}

/// RGB565 to 8 bits a channel by bit replication: a channel's top bits repeat below it. 0 stays
/// 0, full scale reaches 255, and every value lands within one of `value * 255 / full scale`,
/// with no table and no division.
fn widen(pixel: u16) -> [u8; 3] {
    let [red, green, blue] = [pixel >> 11, pixel >> 5 & 0x3f, pixel & 0x1f].map(|c| c as u8);
    [
        red << 3 | red >> 2,
        green << 2 | green >> 4,
        blue << 3 | blue >> 2,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = WIDTH as usize;
    const H: usize = HEIGHT as usize;

    fn flags() -> Flags {
        Flags {
            display_on: true,
            inverted: true,
            reset: false,
            sleep: false,
        }
    }

    /// Five bands of 72 rows, each one RGB565 colour written high byte first: red, green, blue,
    /// white and black, as the board showed them.
    fn bands() -> Vec<u8> {
        let colours: [u16; 5] = [0xf800, 0x07e0, 0x001f, 0xffff, 0x0000];
        (0..H)
            .flat_map(|y| {
                let pixel = colours[y / (H / 5)].to_be_bytes();
                (0..W).flat_map(move |_| pixel)
            })
            .collect()
    }

    fn at(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        pixels[(y * W + x) * RGBA..][..RGBA].try_into().unwrap()
    }

    /// The pixel at the centre column of each band's middle row.
    fn band_middles(pixels: &[u8]) -> Vec<[u8; 4]> {
        (0..5).map(|k| at(pixels, W / 2, k * 72 + 36)).collect()
    }

    #[test]
    fn with_invon_five_bands_show_red_green_blue_white_black() {
        assert_eq!(
            band_middles(&glass(flags(), &bands())),
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
                [0, 0, 0, 255],
            ]
        );
    }

    #[test]
    fn with_invoff_five_bands_show_each_colour_inverted() {
        let off = Flags {
            inverted: false,
            ..flags()
        };
        assert_eq!(
            band_middles(&glass(off, &bands())),
            [
                [0, 255, 255, 255],
                [255, 0, 255, 255],
                [255, 255, 0, 255],
                [0, 0, 0, 255],
                [255, 255, 255, 255],
            ]
        );
    }

    #[test]
    fn two_colours_in_one_row_keep_their_columns() {
        let mut frame = vec![0u8; W * H * 2];
        frame[(40 * W + 170) * 2..][..2].copy_from_slice(&0xf800u16.to_be_bytes());
        frame[(40 * W + 189) * 2..][..2].copy_from_slice(&0x001fu16.to_be_bytes());
        let pixels = glass(flags(), &frame);
        assert_eq!(at(&pixels, 170, 40), [255, 0, 0, 255]);
        assert_eq!(at(&pixels, 189, 40), [0, 0, 255, 255]);
    }

    #[test]
    fn display_off_sleep_and_reset_each_show_opaque_black_on_the_glass() {
        let white = vec![0xff; W * H * 2];
        let dark = [
            Flags {
                display_on: false,
                ..flags()
            },
            Flags {
                sleep: true,
                ..flags()
            },
            Flags {
                reset: true,
                ..flags()
            },
        ];
        for flags in dark {
            let pixels = glass(flags, &white);
            assert_eq!(at(&pixels, W / 2, H / 2), [0, 0, 0, 255], "{flags:?}");
            assert_eq!(at(&pixels, 0, 0), [0, 0, 0, 0], "{flags:?}");
        }
        assert_eq!(at(&glass(flags(), &white), W / 2, H / 2), [255; 4]);
    }

    /// Row 0 holds the 26 pixels from column 167 to 192, and by symmetry so do the last row and
    /// the first and last columns. 101780 of the 129600 pixels are on the glass.
    #[test]
    fn the_glass_is_the_circle_through_the_middle_of_each_edge() {
        let pixels = glass(flags(), &vec![0xff; W * H * 2]);
        let opaque = |x: usize, y: usize| at(&pixels, x, y)[3] == 255;
        let row: Vec<usize> = (0..W).filter(|&x| opaque(x, 0)).collect();
        assert_eq!(row, (167..=192).collect::<Vec<_>>());
        for k in 0..W {
            let edges = [
                opaque(k, 0),
                opaque(k, H - 1),
                opaque(0, k),
                opaque(W - 1, k),
            ];
            assert_eq!(edges, [opaque(k, 0); 4], "edge pixel {k}");
        }
        for y in 0..H {
            for x in 0..W {
                assert_eq!(opaque(x, y), opaque(W - 1 - x, y), "({x}, {y}) mirrors");
                assert_eq!(opaque(x, y), opaque(y, x), "({x}, {y}) transposes");
            }
        }
        assert_eq!(at(&pixels, 166, 0), [0; 4]);
        assert_eq!(at(&pixels, 0, 166), [0; 4]);
        let count = (0..W * H).filter(|k| opaque(k % W, k / W)).count();
        assert_eq!(count, 101780);
    }

    /// Each red and blue value of 5 bits and each green value of 6 bits widens to within one of
    /// its exact share of 255, from 0 to 255, rising with the value. Half scale pins the rule.
    #[test]
    fn each_channel_widens_by_bit_replication() {
        let widened = |channel: usize, full: u16, shift: u16| -> Vec<u8> {
            (0..=full).map(|v| widen(v << shift)[channel]).collect()
        };
        for (channel, full, shift) in [(0, 31, 11), (1, 63, 5), (2, 31, 0)] {
            let values = widened(channel, full, shift);
            assert_eq!((values[0], values[usize::from(full)]), (0, 255));
            assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
            for (v, &got) in values.iter().enumerate() {
                let exact = v as f64 * 255.0 / f64::from(full);
                assert!(
                    (f64::from(got) - exact).abs() < 1.0,
                    "{channel}: {v} gave {got}"
                );
            }
        }
        assert_eq!(widen(0x8410), [0x84, 0x82, 0x84]);
    }
}
