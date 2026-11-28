//! Automatic removal of technical noise before discovery: margin and saturated events, debris, doublets, flow-rate
//! anomalies and dead cells. Each step is optional, reports how many events it removed, and nothing is random.
//!
//! These are the standard cleaning steps (as in flowClean / flowAI / PeacoQC-style pipelines and the usual
//! FSC-A/FSC-H singlet gate), written from their published descriptions. They are meant to keep a map or clustering
//! on biology instead of acquisition artefacts; they are not a substitute for looking at the data.

use crate::comp_wizard::{auto_cut, SplitMethod};
use crate::sample::LoadedSample;
use sf_core::stats;

#[derive(Debug, Clone, PartialEq)]
pub struct DeadSpec {
    /// The viability dye channel.
    pub channel: String,
    /// Events at or above this are dead; `None` = found automatically from the sample.
    pub threshold: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanSpec {
    /// Events pinned at the top of a channel's range (saturated) or at zero on a scatter channel.
    pub margins: bool,
    /// Drop the separate low-FSC population below the cells, when there is one (see [`debris_cut`]).
    pub debris: bool,
    /// FSC-A against FSC-H (or width): drop events whose area is too large for their height.
    pub doublets: bool,
    /// Drop time slices whose event rate is far from the median (clogs, bubbles, restarts).
    pub flow_rate: bool,
    pub dead: Option<DeadSpec>,
}

impl CleanSpec {
    pub fn any(&self) -> bool {
        self.margins || self.debris || self.doublets || self.flow_rate || self.dead.is_some()
    }
}

/// Events kept, and for each step the number it removed (of the events still kept when it ran).
pub fn keep_mask(s: &LoadedSample, spec: &CleanSpec) -> (Vec<bool>, Vec<(&'static str, usize)>) {
    let n = s.n_events;
    let d = s.channels.len();
    let mut keep = vec![true; n];
    let mut steps: Vec<(&'static str, usize)> = Vec::new();
    let col = |c: usize| -> Vec<f64> { (0..n).map(|r| s.events[r * d + c] as f64).collect() };
    let find = |want: &str| s.channels.iter().take(s.n_base).position(|c| c.eq_ignore_ascii_case(want));
    let apply = |name: &'static str, drop: Vec<bool>, keep: &mut Vec<bool>, steps: &mut Vec<(&'static str, usize)>| {
        let mut removed = 0;
        for i in 0..n {
            if keep[i] && drop[i] {
                keep[i] = false;
                removed += 1;
            }
        }
        steps.push((name, removed));
    };

    if spec.margins {
        let mut drop = vec![false; n];
        for c in 0..s.n_base.min(d) {
            let name = s.channels[c].to_ascii_uppercase();
            if name == "TIME" || name.ends_with("-W") {
                continue;
            }
            let range = s.ranges[c];
            let scatter = name.starts_with("FSC") || name.starts_with("SSC");
            for r in 0..n {
                let v = s.events[r * d + c] as f64;
                if (range > 0.0 && v >= range * 0.9999) || (scatter && v <= 0.0) {
                    drop[r] = true;
                }
            }
        }
        apply("margin / saturated events", drop, &mut keep, &mut steps);
    }
    if spec.debris {
        // Debris is what sits BELOW the cells on forward scatter. (This used to keep only the single densest
        // FSC/SSC cluster, which throws away every other cell type, and the cells themselves whenever debris is
        // the densest spot: one real file lost 92 % of its events that way.)
        let mut drop = vec![false; n];
        if let Some(c) = find("FSC-A").or_else(|| find("FSC-H")) {
            let v = col(c);
            let kept: Vec<f64> = (0..n).filter(|&i| keep[i]).map(|i| v[i]).collect();
            if let Some(cut) = debris_cut(&kept) {
                for i in 0..n {
                    drop[i] = v[i] < cut;
                }
            }
        }
        apply("debris (low FSC)", drop, &mut keep, &mut steps);
    }
    if spec.doublets {
        let mut drop = vec![false; n];
        if let (Some(a), Some(h)) = (find("FSC-A"), find("FSC-H")) {
            let (va, vh) = (col(a), col(h));
            let ratio: Vec<f64> = (0..n).map(|i| if vh[i] > 0.0 && va[i] > 0.0 { va[i] / vh[i] } else { f64::NAN }).collect();
            let ok: Vec<f64> = (0..n).filter(|&i| keep[i] && ratio[i].is_finite()).map(|i| ratio[i]).collect();
            if ok.len() > 100 {
                let (med, sd) = (stats::median(&ok), 1.4826 * stats::mad(&ok));
                let limit = med + 3.0 * sd.max(med * 0.02);
                for i in 0..n {
                    drop[i] = ratio[i].is_finite() && ratio[i] > limit;
                }
            }
        } else if let Some(w) = find("FSC-W") {
            let vw = col(w);
            let ok: Vec<f64> = (0..n).filter(|&i| keep[i]).map(|i| vw[i]).collect();
            if ok.len() > 100 {
                let (med, sd) = (stats::median(&ok), 1.4826 * stats::mad(&ok));
                let limit = med + 3.0 * sd.max(med * 0.02);
                for i in 0..n {
                    drop[i] = vw[i] > limit;
                }
            }
        }
        apply("doublets", drop, &mut keep, &mut steps);
    }
    if spec.flow_rate {
        let mut drop = vec![false; n];
        if let Some(t) = find("Time") {
            let tv = col(t);
            let (tmin, tmax) = (tv.iter().cloned().fold(f64::INFINITY, f64::min), tv.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
            if n >= 200 && tmax > tmin {
                let nbins = (n / 200).clamp(10, 100);
                let mut counts = vec![0.0f64; nbins];
                let bin = |v: f64| (((v - tmin) / (tmax - tmin)) * nbins as f64).floor().clamp(0.0, nbins as f64 - 1.0) as usize;
                for &v in &tv {
                    counts[bin(v)] += 1.0;
                }
                let med = stats::median(&counts);
                let mad = stats::mad(&counts).max(1.0);
                let bad: Vec<bool> = counts.iter().map(|c| (c - med).abs() > 3.0 * 1.4826 * mad).collect();
                for i in 0..n {
                    drop[i] = bad[bin(tv[i])];
                }
            }
        }
        apply("flow-rate anomalies", drop, &mut keep, &mut steps);
    }
    if let Some(dead) = &spec.dead {
        let mut drop = vec![false; n];
        if let Some(c) = s.channel_index(&dead.channel) {
            let v = col(c);
            let live: Vec<f64> = (0..n).filter(|&i| keep[i]).map(|i| v[i]).collect();
            let cut = dead.threshold.unwrap_or_else(|| auto_cut(&live, SplitMethod::Auto));
            for i in 0..n {
                drop[i] = v[i] >= cut;
            }
        }
        apply("dead cells", drop, &mut keep, &mut steps);
    }
    (keep, steps)
}

/// The forward-scatter value below which events are debris: the valley between a separate low-FSC population and
/// the cells above it. `None` when the sample has no such separate population, so nothing is removed on a guess.
pub fn debris_cut(fsc: &[f64]) -> Option<f64> {
    let v: Vec<f64> = fsc.iter().copied().filter(|x| x.is_finite() && *x > 0.0).collect();
    if v.len() < 500 {
        return None;
    }
    let hi = stats::percentile(&v, 99.5);
    if !(hi > 0.0) {
        return None;
    }
    const B: usize = 128;
    let mut h = vec![0.0f64; B];
    for x in &v {
        h[((x / hi) * B as f64).floor().clamp(0.0, B as f64 - 1.0) as usize] += 1.0;
    }
    // light smoothing so counting noise does not make peaks
    let s: Vec<f64> = (0..B).map(|i| (i.saturating_sub(1)..=(i + 1).min(B - 1)).map(|k| h[k]).sum::<f64>() / 3.0).collect();
    let top = s.iter().cloned().fold(0.0, f64::max);
    let peaks: Vec<usize> = (0..B).filter(|&i| s[i] >= 0.02 * top && (i == 0 || s[i] >= s[i - 1]) && (i == B - 1 || s[i] > s[i + 1])).collect();
    // debris is the lowest population, in the bottom third of the scale; the cells are the tallest peak above it
    let low = *peaks.first().filter(|&&p| p < B / 3)?;
    let cells = peaks.iter().copied().filter(|&p| p > low + 3).max_by(|a, b| s[*a].total_cmp(&s[*b]))?;
    let valley = (low..=cells).min_by(|a, b| s[*a].total_cmp(&s[*b]))?;
    // a real gap between the two, not a shoulder
    if s[valley] > 0.6 * s[low].min(s[cells]) {
        return None;
    }
    Some((valley as f64 + 0.5) / B as f64 * hi)
}

/// "1,204 events removed (debris 800, doublets 404)" for the log; `None` when nothing was removed.
/// A step that removed more than half of what it was given is flagged, because that is rarely right.
pub fn summary(steps: &[(&'static str, usize)]) -> Option<String> {
    let total: usize = steps.iter().map(|s| s.1).sum();
    (total > 0).then(|| format!("{total} event(s) removed ({})", steps.iter().filter(|s| s.1 > 0).map(|s| format!("{} {}", s.0, s.1)).collect::<Vec<_>>().join(", ")))
}

/// Names of the steps that removed more than half of the `n_events` of a sample.
pub fn heavy_steps(steps: &[(&'static str, usize)], n_events: usize) -> Vec<&'static str> {
    steps.iter().filter(|s| n_events > 0 && s.1 * 2 > n_events).map(|s| s.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::rng::Rng;

    fn normal(rng: &mut Rng) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Time, FSC-A, FSC-H, SSC-A, Viability, Marker. Truth is recorded by event index: 0 = good, bit flags for the rest.
    fn sample() -> (LoadedSample, Vec<u8>) {
        let mut rng = Rng::new(11);
        let n = 20_000;
        let mut ev = Vec::new();
        let mut why = vec![0u8; n];
        for i in 0..n {
            let mut t = i as f64 * 5.0;
            let mut fsc_a = 120_000.0 + 10_000.0 * normal(&mut rng);
            let mut fsc_h = fsc_a * (0.80 + 0.02 * normal(&mut rng));
            let mut ssc = 50_000.0 + 6_000.0 * normal(&mut rng);
            let mut via = 100.0 + 60.0 * normal(&mut rng);
            let marker = 1000.0 + 300.0 * normal(&mut rng);
            match i % 50 {
                1 => {
                    // doublet: twice the area, the same height
                    fsc_a *= 1.9;
                    why[i] |= 1;
                }
                2 => {
                    // debris
                    fsc_a = 9_000.0 + 2000.0 * normal(&mut rng);
                    fsc_h = fsc_a * 0.8;
                    ssc = 4_000.0 + 1000.0 * normal(&mut rng);
                    why[i] |= 2;
                }
                3 => {
                    via = 9_000.0 + 1500.0 * normal(&mut rng);
                    why[i] |= 4;
                }
                4 => {
                    fsc_a = 262_144.0;
                    fsc_h = 210_000.0;
                    why[i] |= 8;
                }
                _ => {}
            }
            // a flow-rate clog: a stretch of time with almost no events (events in it compress in time)
            if (8_000..9_000).contains(&i) {
                t = 40_000.0 + (i - 8_000) as f64 * 0.01 + 0.0;
                why[i] |= 16;
            } else if i >= 9_000 {
                t = i as f64 * 5.0 - 5_000.0 + 10.0;
            }
            ev.extend_from_slice(&[t as f32, fsc_a as f32, fsc_h as f32, ssc as f32, via as f32, marker as f32]);
        }
        let s = LoadedSample::synthetic("c.fcs", &["Time", "FSC-A", "FSC-H", "SSC-A", "Viability-A", "CD3-A"], vec![1e6, 262_144.0, 262_144.0, 262_144.0, 262_144.0, 262_144.0], ev);
        (s, why)
    }

    fn removed_of(why: &[u8], keep: &[bool], flag: u8) -> (usize, usize) {
        let all = why.iter().filter(|&&w| w & flag != 0).count();
        let gone = (0..why.len()).filter(|&i| why[i] & flag != 0 && !keep[i]).count();
        (gone, all)
    }

    #[test]
    fn each_step_removes_its_kind_of_event_and_leaves_most_good_events() {
        let (s, why) = sample();
        for (flag, spec) in [
            (1u8, CleanSpec { doublets: true, ..Default::default() }),
            (2, CleanSpec { debris: true, ..Default::default() }),
            (4, CleanSpec { dead: Some(DeadSpec { channel: "Viability-A".into(), threshold: None }), ..Default::default() }),
            (8, CleanSpec { margins: true, ..Default::default() }),
            (16, CleanSpec { flow_rate: true, ..Default::default() }),
        ] {
            let (keep, steps) = keep_mask(&s, &spec);
            let (gone, all) = removed_of(&why, &keep, flag);
            assert!(gone * 10 >= all * 8, "flag {flag}: removed {gone} of {all} ({steps:?})");
            let good_gone = (0..why.len()).filter(|&i| why[i] == 0 && !keep[i]).count();
            let good = why.iter().filter(|&&w| w == 0).count();
            assert!(good_gone * 10 < good, "flag {flag}: {good_gone} of {good} good events lost ({steps:?})");
        }
    }

    #[test]
    fn all_steps_together_report_what_they_removed_and_nothing_means_no_change() {
        let (s, why) = sample();
        let spec = CleanSpec { margins: true, debris: true, doublets: true, flow_rate: true, dead: Some(DeadSpec { channel: "Viability-A".into(), threshold: Some(2000.0) }) };
        let (keep, steps) = keep_mask(&s, &spec);
        assert_eq!(steps.len(), 5);
        let total: usize = steps.iter().map(|s| s.1).sum();
        assert_eq!(total, keep.iter().filter(|&&k| !k).count(), "each removed event is counted once");
        assert!(summary(&steps).unwrap().contains("doublets"));
        let good_kept = (0..why.len()).filter(|&i| why[i] == 0 && keep[i]).count();
        assert!(good_kept * 10 > why.iter().filter(|&&w| w == 0).count() * 8);
        // an empty spec, and a sample without the needed channels, change nothing
        let (k0, st0) = keep_mask(&s, &CleanSpec::default());
        assert!(k0.iter().all(|&k| k) && st0.is_empty() && !CleanSpec::default().any());
        let bare = LoadedSample::synthetic("b.fcs", &["CD3-A", "CD4-A"], vec![1e5; 2], vec![5.0; 600]);
        let (k1, _) = keep_mask(&bare, &CleanSpec { margins: true, debris: true, doublets: true, flow_rate: true, dead: Some(DeadSpec { channel: "none".into(), threshold: None }) });
        assert!(k1.iter().all(|&k| k));
        assert!(summary(&[("x", 0)]).is_none());
    }

    /// FSC-A, SSC-A, marker: `debris_frac` of the events are debris, the rest are two cell types of different size.
    fn scatter_sample(debris_frac: f64) -> (LoadedSample, Vec<u8>) {
        let mut rng = Rng::new(5);
        let n = 30_000;
        let (mut ev, mut kind) = (Vec::new(), vec![0u8; n]);
        for i in 0..n {
            let u = rng.next_f64();
            let (fsc, ssc) = if u < debris_frac {
                kind[i] = 1;
                (8_000.0 + 2_500.0 * normal(&mut rng), 3_000.0 + 1_000.0 * normal(&mut rng))
            } else if u < debris_frac + (1.0 - debris_frac) * 0.7 {
                (90_000.0 + 9_000.0 * normal(&mut rng), 30_000.0 + 5_000.0 * normal(&mut rng)) // lymphocytes
            } else {
                kind[i] = 2;
                (140_000.0 + 12_000.0 * normal(&mut rng), 90_000.0 + 12_000.0 * normal(&mut rng)) // larger, more granular cells
            };
            ev.extend_from_slice(&[fsc.max(1.0) as f32, ssc.max(1.0) as f32, 500.0]);
        }
        (LoadedSample::synthetic("s.fcs", &["FSC-A", "SSC-A", "CD3-A"], vec![262_144.0; 3], ev), kind)
    }

    #[test]
    fn debris_is_removed_even_when_it_is_the_densest_population_and_other_cell_types_are_kept() {
        for frac in [0.05, 0.4, 0.75] {
            let (s, kind) = scatter_sample(frac);
            let (keep, steps) = keep_mask(&s, &CleanSpec { debris: true, ..Default::default() });
            let frac_kept = |k: u8| (0..kind.len()).filter(|&i| kind[i] == k && keep[i]).count() as f64 / kind.iter().filter(|&&x| x == k).count().max(1) as f64;
            assert!(frac_kept(1) < 0.05, "debris {frac}: {:.3} of the debris kept ({steps:?})", frac_kept(1));
            assert!(frac_kept(0) > 0.98, "debris {frac}: {:.3} of the lymphocytes kept", frac_kept(0));
            assert!(frac_kept(2) > 0.98, "debris {frac}: the second, larger cell type is not thrown away ({:.3} kept)", frac_kept(2));
        }
        // 75 % removed is correct there, and is flagged for the user to look at
        let (s, _) = scatter_sample(0.75);
        let (_, steps) = keep_mask(&s, &CleanSpec { debris: true, ..Default::default() });
        assert_eq!(heavy_steps(&steps, s.n_events), vec!["debris (low FSC)"]);
    }

    #[test]
    fn a_sample_without_a_separate_debris_population_loses_nothing() {
        let (s, _) = scatter_sample(0.0);
        let (keep, steps) = keep_mask(&s, &CleanSpec { debris: true, ..Default::default() });
        assert!(keep.iter().all(|&k| k), "{steps:?}");
        assert!(heavy_steps(&steps, s.n_events).is_empty());
        // one broad population with a low shoulder is not cut either
        let mut rng = Rng::new(9);
        let one: Vec<f64> = (0..20_000).map(|_| 60_000.0 + 25_000.0 * normal(&mut rng)).filter(|v| *v > 0.0).collect();
        assert_eq!(debris_cut(&one), None);
        assert_eq!(debris_cut(&[1.0; 10]), None, "too few events to judge");
    }
}
