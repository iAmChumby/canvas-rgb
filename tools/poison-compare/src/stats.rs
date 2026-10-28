//! Statistics for the Poison comparison. Both sides (Chromium's render and the Rust
//! runtime's render) go through these same functions, so any difference in the numbers
//! comes from the pixels.
//!
//! Color distance is CIE76 ΔE (Euclidean distance in CIELAB, D65 white). CIE76 is the
//! metric the brief allows; CIEDE2000 would track perception more closely but is not needed
//! to settle a 15-unit threshold on 4 palette colors.

/// A pixel is "on" a palette color when its nearest palette color is that one and it is
/// within this ΔE of it.
pub const PALETTE_DELTA_E: f64 = 15.0;

/// Channel means must agree within this fraction of the original mean (relative tolerance).
pub const MEAN_RELATIVE_TOLERANCE: f64 = 0.10;

/// Below this original mean (0..255 units), the channel uses `MEAN_ABSOLUTE_TOLERANCE`.
pub const MEAN_SMALL_THRESHOLD: f64 = 5.0;
pub const MEAN_ABSOLUTE_TOLERANCE: f64 = 5.0;

/// Palette shares must agree within this many percentage points.
pub const SHARE_TOLERANCE_PP: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lab_f(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

/// sRGB 8-bit to CIELAB (D65).
pub fn srgb_to_lab(px: [u8; 3]) -> Lab {
    let r = srgb_to_linear(px[0]);
    let g = srgb_to_linear(px[1]);
    let b = srgb_to_linear(px[2]);
    let x = 0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = 0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b;
    let fx = lab_f(x / 0.950_47);
    let fy = lab_f(y);
    let fz = lab_f(z / 1.088_83);
    Lab { l: 116.0 * fy - 16.0, a: 500.0 * (fx - fy), b: 200.0 * (fy - fz) }
}

pub fn delta_e76(x: Lab, y: Lab) -> f64 {
    let (dl, da, db) = (x.l - y.l, x.a - y.a, x.b - y.b);
    (dl * dl + da * da + db * db).sqrt()
}

/// Statistics of one frame (one run's final canvas).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// Mean R, G, B over all pixels, 0..255.
    pub mean: [f64; 3],
    /// Share of pixels that are on palette color `i` (nearest and within ΔE 15), 0..1.
    pub share: Vec<f64>,
    /// Share of pixels not within ΔE 15 of their nearest palette color, 0..1.
    pub other: f64,
}

/// `pixels` is packed RGB, 3 bytes per pixel. `palette` may hold any number of colors.
/// Returns `None` for an empty frame or empty palette (an empty denominator is not a value).
pub fn frame_stats(pixels: &[u8], palette: &[[u8; 3]]) -> Option<FrameStats> {
    if pixels.is_empty() || !pixels.len().is_multiple_of(3) || palette.is_empty() {
        return None;
    }
    let (pixel_chunks, _) = pixels.as_chunks::<3>();
    let n = pixel_chunks.len() as f64;
    let palette_lab: Vec<Lab> = palette.iter().map(|&c| srgb_to_lab(c)).collect();
    let mut sums = [0.0f64; 3];
    let mut counts = vec![0u64; palette.len()];
    let mut other = 0u64;

    for &rgb in pixel_chunks {
        for (sum, value) in sums.iter_mut().zip(rgb) {
            *sum += f64::from(value);
        }
        let lab = srgb_to_lab(rgb);
        let (mut best, mut best_de) = (0usize, f64::INFINITY);
        for (i, p) in palette_lab.iter().enumerate() {
            let de = delta_e76(lab, *p);
            if de < best_de {
                best = i;
                best_de = de;
            }
        }
        if best_de <= PALETTE_DELTA_E {
            counts[best] += 1;
        } else {
            other += 1;
        }
    }

    Some(FrameStats {
        mean: [sums[0] / n, sums[1] / n, sums[2] / n],
        share: counts.iter().map(|&c| c as f64 / n).collect(),
        other: other as f64 / n,
    })
}

/// Mean and sample standard deviation over runs. `sd` is `None` with fewer than 2 runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agg {
    pub mean: f64,
    pub sd: Option<f64>,
}

pub fn aggregate(values: &[f64]) -> Option<Agg> {
    if values.is_empty() {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let sd = if values.len() < 2 {
        None
    } else {
        let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0);
        Some(var.sqrt())
    };
    Some(Agg { mean, sd })
}

/// Per-side result: each statistic aggregated across runs.
#[derive(Debug, Clone, PartialEq)]
pub struct SideSummary {
    pub runs: usize,
    pub mean: [Agg; 3],
    pub share: Vec<Agg>,
    pub other: Agg,
}

fn column<F: Fn(&FrameStats) -> f64>(frames: &[FrameStats], f: F) -> Vec<f64> {
    frames.iter().map(f).collect()
}

/// Aggregates per-run frame statistics. Returns `None` if there are no runs or the runs
/// disagree on palette size.
pub fn summarize(frames: &[FrameStats]) -> Option<SideSummary> {
    let palette_len = frames.first()?.share.len();
    if frames.iter().any(|f| f.share.len() != palette_len) {
        return None;
    }
    let share = (0..palette_len)
        .map(|i| aggregate(&column(frames, |f| f.share[i])))
        .collect::<Option<Vec<_>>>()?;
    Some(SideSummary {
        runs: frames.len(),
        mean: [
            aggregate(&column(frames, |f| f.mean[0]))?,
            aggregate(&column(frames, |f| f.mean[1]))?,
            aggregate(&column(frames, |f| f.mean[2]))?,
        ],
        share,
        other: aggregate(&column(frames, |f| f.other))?,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub original: f64,
    pub port: f64,
    /// Allowed |port - original|, in the same unit as the values.
    pub tolerance: f64,
    pub delta: f64,
    pub pass: bool,
}

/// Channel mean check. Values are 0..255. Tolerance is 10% of the original mean, or an
/// absolute 5 when the original mean is below 5.
pub fn check_mean(name: &str, original: f64, port: f64) -> Check {
    let tolerance = if original < MEAN_SMALL_THRESHOLD {
        MEAN_ABSOLUTE_TOLERANCE
    } else {
        MEAN_RELATIVE_TOLERANCE * original
    };
    let delta = port - original;
    Check {
        name: name.to_string(),
        original,
        port,
        tolerance,
        delta,
        pass: delta.abs() <= tolerance,
    }
}

/// Palette share check. Inputs are fractions 0..1; the tolerance and delta are in percentage points.
pub fn check_share(name: &str, original: f64, port: f64) -> Check {
    let delta = (port - original) * 100.0;
    Check {
        name: name.to_string(),
        original: original * 100.0,
        port: port * 100.0,
        tolerance: SHARE_TOLERANCE_PP,
        delta,
        // Epsilon absorbs float error in (port - original) * 100 at exactly the limit.
        pass: delta.abs() <= SHARE_TOLERANCE_PP + 1e-9,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn lab_reference_points() {
        let white = srgb_to_lab([255, 255, 255]);
        assert!(close(white.l, 100.0, 0.01), "white L {}", white.l);
        assert!(close(white.a, 0.0, 0.01) && close(white.b, 0.0, 0.01));
        let black = srgb_to_lab([0, 0, 0]);
        assert!(close(black.l, 0.0, 1e-9));
        // Published D65 CIELAB values for sRGB red.
        let red = srgb_to_lab([255, 0, 0]);
        assert!(close(red.l, 53.24, 0.05), "red L {}", red.l);
        assert!(close(red.a, 80.09, 0.05), "red a {}", red.a);
        assert!(close(red.b, 67.20, 0.05), "red b {}", red.b);
    }

    #[test]
    fn delta_e_identity_and_symmetry() {
        let x = srgb_to_lab([0x60, 0x00, 0xfc]);
        let y = srgb_to_lab([0xb3, 0x00, 0xff]);
        assert_eq!(delta_e76(x, x), 0.0);
        assert!(close(delta_e76(x, y), delta_e76(y, x), 1e-12));
        assert!(delta_e76(x, y) > 0.0);
    }

    #[test]
    fn shares_split_nearest_and_far_pixels() {
        let black = [0, 0, 0];
        let white = [255, 255, 255];
        let red = [255, 0, 0];
        // 3 black, 1 near-white (ΔE well under 15), 1 mid-gray (ΔE far from every color).
        let pixels = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, // three black
            250, 252, 251, // near white
            128, 128, 128, // gray: nearest is black or white, but far from both
        ];
        let stats = frame_stats(&pixels, &[black, white, red]).unwrap();
        assert!(close(stats.share[0], 3.0 / 5.0, 1e-12), "black {:?}", stats.share);
        assert!(close(stats.share[1], 1.0 / 5.0, 1e-12), "white {:?}", stats.share);
        assert!(close(stats.share[2], 0.0, 1e-12));
        assert!(close(stats.other, 1.0 / 5.0, 1e-12), "other {}", stats.other);
        let total: f64 = stats.share.iter().sum::<f64>() + stats.other;
        assert!(close(total, 1.0, 1e-12));
        assert!(close(stats.mean[0], (250.0 + 128.0) / 5.0, 1e-9));
    }

    #[test]
    fn empty_inputs_are_none_not_zero() {
        assert!(frame_stats(&[], &[[0, 0, 0]]).is_none());
        assert!(frame_stats(&[1, 2, 3], &[]).is_none());
        assert!(frame_stats(&[1, 2], &[[0, 0, 0]]).is_none());
        assert!(summarize(&[]).is_none());
    }

    #[test]
    fn aggregate_sd_needs_two_runs() {
        assert_eq!(aggregate(&[4.0]).unwrap(), Agg { mean: 4.0, sd: None });
        let a = aggregate(&[2.0, 4.0]).unwrap();
        assert!(close(a.mean, 3.0, 1e-12));
        assert!(close(a.sd.unwrap(), 2f64.sqrt(), 1e-12));
    }

    #[test]
    fn mean_check_relative_and_small_absolute() {
        // 10% of 100 is 10: 109 passes, 111 fails.
        assert!(check_mean("R", 100.0, 109.0).pass);
        assert!(!check_mean("R", 100.0, 111.0).pass);
        // Original 3.0 is below 5: absolute tolerance of 5 applies, so 7.9 passes, 8.5 fails.
        let small = check_mean("G", 3.0, 7.9);
        assert!(small.pass && close(small.tolerance, 5.0, 1e-12));
        assert!(!check_mean("G", 3.0, 8.5).pass);
    }

    #[test]
    fn share_check_in_percentage_points() {
        assert!(check_share("bg", 0.30, 0.40).pass); // exactly 10.0 pp
        assert!(!check_share("bg", 0.30, 0.41).pass); // 11 pp
        let c = check_share("bg", 0.5, 0.45);
        assert!(close(c.delta, -5.0, 1e-9) && close(c.original, 50.0, 1e-9));
    }
}
