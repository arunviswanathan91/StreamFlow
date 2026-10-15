//! Cross-Sample analysis: how each population behaves across samples. The numbers behind the Cross-Sample and
//! Classifier screens.
//!
//! * the population x sample frequency matrix (% of all events), keyed by the population's full path so two
//!   populations that merely share a name in different branches are never confused;
//! * the consistency check per population (mean, SD, CV% over the samples that have it, flagged above 50% CV);
//! * the radar (each axis scaled by its largest value across samples) and the per-channel MFI drift.
//!
//! The SD is the population SD like every other statistic in the project (the Java screen used the sample SD).

use crate::session::Session;
use crate::stats_table::{self, Metric};
use sf_core::stats;
use sf_gating::GateTree;

/// Frequency of each population in each selected sample; `NaN` where the sample has no such population.
#[derive(Debug, Clone)]
pub struct FreqMatrix {
    pub populations: Vec<Vec<String>>,
    pub samples: Vec<usize>,
    pub sample_names: Vec<String>,
    /// `freq[population][sample]`, percent of the sample's events.
    pub freq: Vec<Vec<f64>>,
}

pub fn frequency_matrix(session: &mut Session, samples: &[usize]) -> FreqMatrix {
    let table = stats_table::compute(session, samples, &[Metric::PctTotal], None);
    let sample_names: Vec<String> = samples.iter().map(|&i| session.sample(i).name.clone()).collect();
    let mut populations: Vec<Vec<String>> = Vec::new();
    for r in &table.rows {
        if !r.population.is_empty() && !populations.contains(&r.population) {
            populations.push(r.population.clone());
        }
    }
    let mut freq = vec![vec![f64::NAN; samples.len()]; populations.len()];
    for r in &table.rows {
        if r.population.is_empty() {
            continue;
        }
        let (Some(pi), Some(si)) = (populations.iter().position(|p| *p == r.population), sample_names.iter().position(|n| *n == r.sample)) else { continue };
        if let Some(v) = r.values[0] {
            freq[pi][si] = v;
        }
    }
    FreqMatrix { populations, samples: samples.to_vec(), sample_names, freq }
}

#[derive(Debug, Clone)]
pub struct Consistency {
    pub population: Vec<String>,
    /// Samples that have the population.
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
    pub cv: f64,
    pub consistent: bool,
}

/// CV above this is flagged as inconsistent.
pub const CV_LIMIT: f64 = 50.0;

pub fn consistency(m: &FreqMatrix) -> Vec<Consistency> {
    m.populations
        .iter()
        .zip(&m.freq)
        .map(|(pop, row)| {
            let v: Vec<f64> = row.iter().copied().filter(|x| x.is_finite()).collect();
            let (mean, sd) = if v.is_empty() { (f64::NAN, f64::NAN) } else { (stats::mean(&v), stats::sd(&v)) };
            let cv = if mean > 1e-9 { 100.0 * sd / mean } else { 0.0 };
            Consistency { population: pop.clone(), n: v.len(), mean, sd, cv, consistent: !(cv > CV_LIMIT) }
        })
        .collect()
}

/// z-scores of a column for the heat colouring (0 when there is no spread).
pub fn z_scores(values: &[f64]) -> Vec<f64> {
    let v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 2 {
        return vec![0.0; values.len()];
    }
    let (m, s) = (stats::mean(&v), stats::sd(&v));
    values.iter().map(|x| if s > 1e-9 && x.is_finite() { (x - m) / s } else { 0.0 }).collect()
}

#[derive(Debug, Clone)]
pub struct Radar {
    pub axes: Vec<Vec<String>>,
    /// `fractions[sample][axis]` in 0..=1 (a missing population counts as 0).
    pub fractions: Vec<Vec<f64>>,
}

/// The first `max_axes` populations as radar axes, each scaled by its maximum over the samples. `None` with fewer
/// than three axes (a radar needs a polygon).
pub fn radar(m: &FreqMatrix, max_axes: usize) -> Option<Radar> {
    let k = m.populations.len().min(max_axes);
    if k < 3 || m.samples.is_empty() {
        return None;
    }
    let max: Vec<f64> = (0..k).map(|a| m.freq[a].iter().copied().filter(|v| v.is_finite()).fold(0.0, f64::max)).map(|v| if v <= 0.0 { 1.0 } else { v }).collect();
    let fractions = (0..m.samples.len()).map(|s| (0..k).map(|a| if m.freq[a][s].is_finite() { m.freq[a][s] / max[a] } else { 0.0 }).collect()).collect();
    Some(Radar { axes: m.populations[..k].to_vec(), fractions })
}

/// Fluorescence channels of a sample (not scatter, not time).
pub fn fluorescence_channels(channels: &[String]) -> Vec<String> {
    let lower = |c: &String| c.to_lowercase();
    let f: Vec<String> = channels.iter().filter(|c| !(lower(c).contains("fsc") || lower(c).contains("ssc") || lower(c).contains("time"))).cloned().collect();
    if f.is_empty() { channels.to_vec() } else { f }
}

/// Median of each channel for one population in each sample: `[sample][channel]`, NaN where the sample lacks the
/// population or the channel.
pub fn mfi_drift(session: &mut Session, samples: &[usize], population: &[String], channels: &[String]) -> Vec<Vec<f64>> {
    samples
        .iter()
        .map(|&si| {
            let node = if population.is_empty() {
                Some(GateTree::ROOT)
            } else {
                let refs: Vec<&str> = population.iter().map(|s| s.as_str()).collect();
                session.tree(si).find(&refs)
            };
            channels
                .iter()
                .map(|c| match node {
                    Some(n) => session.population_values(si, n, c).ok().filter(|v| !v.is_empty()).map_or(f64::NAN, |v| stats::median(&v)),
                    None => f64::NAN,
                })
                .collect()
        })
        .collect()
}

/// Classifier features: one row per sample, one column per ticked population, frequency in % of all events. A
/// population a sample does not have counts as 0 (the engine does the same).
pub fn feature_matrix(m: &FreqMatrix, use_pop: &[bool]) -> (Vec<String>, Vec<f64>) {
    let cols: Vec<usize> = (0..m.populations.len()).filter(|&i| use_pop.get(i).copied().unwrap_or(true)).collect();
    let names = cols.iter().map(|&i| crate::stats_table::pop_label(&m.populations[i])).collect();
    let mut flat = Vec::with_capacity(m.samples.len() * cols.len());
    for s in 0..m.samples.len() {
        for &c in &cols {
            let v = m.freq[c][s];
            flat.push(if v.is_finite() { v } else { 0.0 });
        }
    }
    (names, flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::sample::LoadedSample;

    /// Three samples of 1000 events on X (0..1000) and Y; gate "Low" takes x<200, 300 and 100 events resp.; sample 2 has no gate "High".
    fn session() -> Session {
        let mut s = Session::new();
        for (k, name) in ["a.fcs", "b.fcs", "c.fcs"].iter().enumerate() {
            let ev: Vec<f32> = (0..1000).flat_map(|i| [i as f32, (i % 50) as f32 * (k + 1) as f32]).collect();
            s.add_sample(LoadedSample::synthetic(name, &["X-A", "Y-A"], vec![1000.0, 1000.0], ev));
        }
        let cut = [200.0, 300.0, 100.0];
        for si in 0..3 {
            let w = s.open_population(si, GateTree::ROOT).unwrap();
            s.window_mut(w).unwrap().x_chan = "X-A".into();
            s.window_mut(w).unwrap().y_chan = "Y-A".into();
            s.window_mut(w).unwrap().x_scale = sf_core::scale::ScaleSpec::Identity; // shapes are in display space: keep it raw
            s.window_mut(w).unwrap().y_scale = sf_core::scale::ScaleSpec::Identity;
            s.commit_shape(w, &Shape::Rect { x0: -1.0, x1: cut[si] - 0.5, y0: -1.0, y1: 1e6 }, "Low").unwrap();
            if si != 2 {
                s.commit_shape(w, &Shape::Rect { x0: 799.5, x1: 1001.0, y0: -1.0, y1: 1e6 }, "High").unwrap();
            }
        }
        s
    }

    #[test]
    fn matrix_consistency_radar_and_features() {
        let mut s = session();
        let m = frequency_matrix(&mut s, &[0, 1, 2]);
        assert_eq!(m.populations, vec![vec!["Low".to_string()], vec!["High".to_string()]]);
        assert_eq!(m.freq[0], vec![20.0, 30.0, 10.0]);
        assert_eq!(&m.freq[1][..2], &[20.0, 20.0]);
        assert!(m.freq[1][2].is_nan(), "a sample without the population is NaN, not 0");
        let c = consistency(&m);
        assert!((c[0].mean - 20.0).abs() < 1e-12 && c[0].n == 3);
        // population SD of 20, 30, 10: sqrt(200/3)
        assert!((c[0].sd - (200.0f64 / 3.0).sqrt()).abs() < 1e-12, "{}", c[0].sd);
        assert!((c[0].cv - 100.0 * (200.0f64 / 3.0).sqrt() / 20.0).abs() < 1e-9 && c[0].consistent);
        assert_eq!((c[1].n, c[1].sd, c[1].consistent), (2, 0.0, true), "only the two samples that have it count");
        // radar needs three axes
        assert!(radar(&m, 12).is_none());
        let mut m3 = m.clone();
        m3.populations.push(vec!["Third".into()]);
        m3.freq.push(vec![0.0, 5.0, 10.0]);
        let r = radar(&m3, 12).unwrap();
        assert_eq!(r.fractions[1][0], 1.0, "the largest value on an axis is 1");
        assert!((r.fractions[0][0] - 20.0 / 30.0).abs() < 1e-12);
        assert_eq!(r.fractions[2][1], 0.0, "a missing population is drawn as 0");
        assert_eq!(r.fractions[2][2], 1.0);
        // classifier features: missing is 0
        let (names, flat) = feature_matrix(&m, &[true, true]);
        assert_eq!(names, vec!["Low", "High"]);
        assert_eq!(flat, vec![20.0, 20.0, 30.0, 20.0, 10.0, 0.0]);
        let (names, flat) = feature_matrix(&m, &[false, true]);
        assert_eq!((names, flat), (vec!["High".to_string()], vec![20.0, 20.0, 0.0]));
    }

    #[test]
    fn a_wildly_varying_population_is_flagged_and_z_scores_are_population_z() {
        let m = FreqMatrix { populations: vec![vec!["P".into()]], samples: vec![0, 1, 2, 3], sample_names: vec!["a".into(), "b".into(), "c".into(), "d".into()], freq: vec![vec![1.0, 1.0, 1.0, 40.0]] };
        let c = consistency(&m);
        assert!(c[0].cv > CV_LIMIT && !c[0].consistent, "{}", c[0].cv);
        let z = z_scores(&[1.0, 1.0, 1.0, 40.0, f64::NAN]);
        assert!(z[3] > 1.5 && z[4] == 0.0);
        assert_eq!(z_scores(&[5.0, 5.0, 5.0]), vec![0.0; 3], "no spread, no colour");
        assert_eq!(z_scores(&[1.0]), vec![0.0]);
    }

    #[test]
    fn mfi_drift_is_the_median_of_each_channel_in_the_population() {
        let mut s = session();
        let chans = vec!["X-A".to_string(), "Y-A".to_string(), "Nope".to_string()];
        let d = mfi_drift(&mut s, &[0, 1, 2], &["High".to_string()], &chans);
        // sample 0's High = x in 800..999: median 899.5
        assert!((d[0][0] - 899.5).abs() < 1e-9, "{:?}", d[0]);
        assert!(d[0][2].is_nan(), "an unknown channel is NaN");
        assert!(d[2].iter().all(|v| v.is_nan()), "a sample without the population is NaN throughout");
        let all = mfi_drift(&mut s, &[1], &[], &chans[..1]);
        assert!((all[0][0] - 499.5).abs() < 1e-9);
        assert_eq!(fluorescence_channels(&["FSC-A".into(), "SSC-A".into(), "CD3-A".into(), "Time".into()]), vec!["CD3-A"]);
        assert_eq!(fluorescence_channels(&["FSC-A".into()]), vec!["FSC-A"]);
    }
}
