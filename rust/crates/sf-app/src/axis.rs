//! Axis ticks and labels for linear and scaled (arcsinh / logicle / biex / log) axes.

use sf_core::scale::ScaleSpec;

#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// Position in display space.
    pub pos: f64,
    pub label: String,
    /// Major ticks are labelled; minor ticks are only drawn.
    pub major: bool,
}

/// Compact label for a raw data value: `0`, `500`, `1k`, `10k`, `-1k`, `1M`.
pub fn format_raw(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    let sign = if v < 0.0 { "-" } else { "" };
    let trim = |x: f64| {
        let s = format!("{x:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    if a >= 1e6 {
        format!("{sign}{}M", trim(a / 1e6))
    } else if a >= 1e3 {
        format!("{sign}{}k", trim(a / 1e3))
    } else if a >= 1.0 || (a - a.round()).abs() < 1e-9 {
        format!("{sign}{}", trim(a))
    } else {
        format!("{sign}{}", trim(a))
    }
}

/// A "nice" step (1, 2 or 5 times a power of ten) giving roughly `target` intervals over `span`.
fn nice_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    mag * if f < 1.5 { 1.0 } else if f < 3.5 { 2.0 } else if f < 7.5 { 5.0 } else { 10.0 }
}

/// Ticks for an axis showing display-space range `[d0, d1]` on `scale`.
///
/// Linear axes get evenly spaced nice values. Scaled axes get the decade structure people read on
/// log-like cytometry axes: 0 and +/-1, 10, 100, ... (major, labelled) with 2..9 x decade as minor
/// ticks, each placed at its transformed position and kept only if it falls inside the range.
pub fn ticks(scale: &ScaleSpec, d0: f64, d1: f64) -> Vec<Tick> {
    if !(d1 > d0) || !d0.is_finite() || !d1.is_finite() {
        return Vec::new();
    }
    let xf = match scale.build() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match scale {
        ScaleSpec::Identity | ScaleSpec::Linear { .. } => {
            // linear in display space; label with raw values (a Linear{min,max} axis maps back)
            let step = nice_step(d1 - d0, 6);
            let first = (d0 / step).ceil() * step;
            let mut out = Vec::new();
            let mut k = 0;
            loop {
                let pos = first + k as f64 * step;
                if pos > d1 + step * 1e-9 || k > 200 {
                    break;
                }
                let pos = if pos.abs() < step * 1e-9 { 0.0 } else { pos };
                out.push(Tick { pos, label: format_raw(xf.inverse(pos)), major: true });
                k += 1;
            }
            out
        }
        _ => {
            let (r0, r1) = (xf.inverse(d0), xf.inverse(d1));
            let hi = r0.abs().max(r1.abs()).max(1.0);
            let top = hi.log10().ceil() as i32 + 1;
            let mut out = Vec::new();
            let mut push = |raw: f64, major: bool| {
                let pos = xf.forward(raw);
                if pos.is_finite() && pos >= d0 && pos <= d1 {
                    out.push(Tick { pos, label: if major { format_raw(raw) } else { String::new() }, major });
                }
            };
            push(0.0, true);
            for e in 0..=top {
                let decade = 10f64.powi(e);
                for sign in [1.0, -1.0] {
                    if e >= 1 || sign > 0.0 {
                        push(sign * decade, e >= 1 && e % 1 == 0);
                    }
                    if e >= 1 {
                        for m in 2..=9 {
                            push(sign * decade * m as f64, false);
                        }
                    }
                }
            }
            out.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap());
            out.dedup_by(|a, b| (a.pos - b.pos).abs() < 1e-12);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_compact() {
        for (v, s) in [(0.0, "0"), (500.0, "500"), (1000.0, "1k"), (12_500.0, "12.5k"), (-1000.0, "-1k"), (2_000_000.0, "2M"), (0.5, "0.5")] {
            assert_eq!(format_raw(v), s);
        }
    }

    #[test]
    fn linear_ticks_are_nice_and_inside_the_range() {
        let t = ticks(&ScaleSpec::Identity, 0.0, 262_144.0);
        assert!(t.len() >= 4 && t.len() <= 12, "{} ticks", t.len());
        assert!(t.iter().all(|k| k.pos >= 0.0 && k.pos <= 262_144.0));
        let step = t[1].pos - t[0].pos;
        assert!(t.windows(2).all(|w| ((w[1].pos - w[0].pos) - step).abs() < 1e-6), "even spacing");
        assert_eq!(t[0].label, "0");
        assert!(ticks(&ScaleSpec::Identity, 5.0, 5.0).is_empty() && ticks(&ScaleSpec::Identity, f64::NAN, 1.0).is_empty());
    }

    #[test]
    fn scaled_axes_show_zero_and_decades_at_their_transformed_positions() {
        let sc = ScaleSpec::Logicle { t: 262144.0, w: 0.5, m: 4.5, a: 0.0 };
        let xf = sc.build().unwrap();
        let (d0, d1) = (xf.forward(-1000.0), xf.forward(262_144.0));
        let t = ticks(&sc, d0, d1);
        let major: Vec<&Tick> = t.iter().filter(|k| k.major).collect();
        let labels: Vec<&str> = major.iter().map(|k| k.label.as_str()).collect();
        for want in ["-1k", "0", "1k", "10k", "100k"] {
            assert!(labels.contains(&want), "missing {want} in {labels:?}");
        }
        // each tick sits where the transform puts its value
        let k1 = major.iter().find(|k| k.label == "1k").unwrap();
        assert!((k1.pos - xf.forward(1000.0)).abs() < 1e-12);
        assert!(t.iter().filter(|k| !k.major).count() > 5, "minor ticks present");
        assert!(t.windows(2).all(|w| w[0].pos < w[1].pos), "sorted and unique");
        assert!(t.iter().all(|k| k.pos >= d0 && k.pos <= d1));
    }

    #[test]
    fn arcsinh_axis_and_degenerate_ranges() {
        let sc = ScaleSpec::Arcsinh { cofactor: 150.0 };
        let xf = sc.build().unwrap();
        let t = ticks(&sc, xf.forward(-500.0), xf.forward(100_000.0));
        assert!(t.iter().any(|k| k.label == "0") && t.iter().any(|k| k.label == "10k"));
        assert!(ticks(&ScaleSpec::Arcsinh { cofactor: -1.0 }, 0.0, 1.0).is_empty(), "invalid scale yields no ticks, not a panic");
    }
}
