//! Equalizer presets and frequency response.

use crate::devices::Band;

pub const F_MIN: f32 = 20.0;
pub const F_MAX: f32 = 20_000.0;
/// Per-band gain limit: never exceeded, even though the graph shows ±12 dB.
pub const GAIN_LIMIT: f32 = 9.0;
pub const Q_MIN: f32 = 0.2;
pub const Q_MAX: f32 = 5.0;
const FS: f64 = 48_000.0;

/// Octave bands for the presets.
const GRID: [f32; 10] = [32., 64., 125., 250., 500., 1000., 2000., 4000., 8000., 16000.];

/// 10-band peaking preset (Q 1) on the octave bands, with these gains in dB.
const fn octaves(gains: [f32; 10]) -> [Band; 10] {
    let mut bands = [(0., 0., 1., 0); 10];
    let mut i = 0;
    while i < 10 {
        bands[i] = (GRID[i], gains[i], 1., 0);
        i += 1;
    }
    bands
}

/// Built-in presets, in the order of `i18n::Tr::presets`. The first one is flat.
pub const PRESETS: [[Band; 10]; 9] = [
    octaves([0., 0., 0., 0., 0., 0., 0., 0., 0., 0.]),
    // Bass
    octaves([5., 5., 3., 1., 0., 0., 0., 0., 0., 0.]),
    // Footsteps: less bass, more presence
    octaves([-3., -3., -2., -1., 0., 1., 3., 5., 3., 0.]),
    // Voice
    octaves([-4., -3., -1., 0., 1., 3., 4., 2., 0., -1.]),
    // Music: gentle V
    octaves([4., 3., 1., 0., -1., -1., 0., 2., 3., 3.]),
    // Cinema
    octaves([4., 4., 2., 0., 0., 1., 2., 1., 1., 0.]),
    // Bright
    octaves([0., 0., 0., 0., 0., 0., 1., 3., 4., 4.]),
    // Warm
    octaves([2., 2., 2., 2., 1., 0., -1., -2., -3., -3.]),
    // Soft: calmer treble for long sessions
    octaves([0., 0., 0., 0., 0., 0., -1., -3., -4., -4.]),
];

/// RBJ coefficients (Audio EQ Cookbook): (b0, b1, b2, a0, a1, a2).
fn biquad((f0, gain, q, kind): Band) -> [f64; 6] {
    let a = 10f64.powf(gain as f64 / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * f0 as f64 / FS;
    let (cw, alpha) = (w0.cos(), w0.sin() / (2.0 * q as f64));
    if kind == 1 {
        let sa = 2.0 * a.sqrt() * alpha;
        [
            a * ((a + 1.0) - (a - 1.0) * cw + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
            a * ((a + 1.0) - (a - 1.0) * cw - sa),
            (a + 1.0) + (a - 1.0) * cw + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cw),
            (a + 1.0) + (a - 1.0) * cw - sa,
        ]
    } else {
        [1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a]
    }
}

/// Total gain in dB of all bands at each frequency.
pub fn response_db(bands: &[Band], freqs: &[f32]) -> Vec<f32> {
    let mut total = vec![0f32; freqs.len()];
    for &band in bands {
        if band.2 <= 0.0 || band.1.abs() < 1e-3 {
            continue;
        }
        let c = biquad(band);
        for (i, &f) in freqs.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * f as f64 / FS;
            // H(e^jw) = (b0 + b1 z^-1 + b2 z^-2) / (a0 + a1 z^-1 + a2 z^-2)
            let (c1, s1, c2, s2) = (w.cos(), -w.sin(), (2.0 * w).cos(), -(2.0 * w).sin());
            let (nr, ni) = (c[0] + c[1] * c1 + c[2] * c2, c[1] * s1 + c[2] * s2);
            let (dr, di) = (c[3] + c[4] * c1 + c[5] * c2, c[4] * s1 + c[5] * s2);
            let mag2 = (nr * nr + ni * ni) / (dr * dr + di * di);
            total[i] += (10.0 * mag2.max(1e-18).log10()) as f32;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaking_gain_at_centre() {
        let r = response_db(&[(1000.0, 6.0, 1.0, 0)], &[100.0, 1000.0, 10_000.0]);
        assert!((r[1] - 6.0).abs() < 0.01, "{r:?}");
        assert!(r[0].abs() < 0.2 && r[2].abs() < 0.2, "{r:?}");
    }

    #[test]
    fn low_shelf_half_gain_at_corner() {
        let r = response_db(&[(100.0, 6.0, 0.7, 1)], &[20.0, 100.0, 1000.0]);
        assert!((r[0] - 6.0).abs() < 0.1 && (r[1] - 3.0).abs() < 0.1 && r[2].abs() < 0.1, "{r:?}");
    }

    #[test]
    fn presets_stay_in_range() {
        assert!(PRESETS[0].iter().all(|b| b.1 == 0.0));
        for bands in PRESETS {
            assert!(bands.iter().all(|&(hz, db, q, _)| (F_MIN..=F_MAX).contains(&hz)
                && db.abs() <= GAIN_LIMIT
                && (Q_MIN..=Q_MAX).contains(&q)));
        }
        assert_eq!(PRESETS.len(), crate::i18n::t().presets.len());
    }
}
