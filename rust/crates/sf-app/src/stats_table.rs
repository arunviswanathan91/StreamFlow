//! The Statistics module's engine: one row per (sample, population), one column per statistic.
//! All numerics come from `sf_core::stats` (one place, by project convention): population SD, type-7
//! percentiles, geometric mean over x > 0 with the excluded count reported.

use crate::session::Session;
use sf_core::stats;
use sf_gating::GateTree;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Metric {
    Count,
    PctParent,
    PctTotal,
    Median,
    Mean,
    GeoMean,
    Mode,
    Sd,
    Cv,
    RobustCv,
    Mad,
    Min,
    Max,
    Percentile(f64),
}

impl Metric {
    /// Statistics that need a channel (everything but the counts).
    pub fn needs_channel(self) -> bool {
        !matches!(self, Metric::Count | Metric::PctParent | Metric::PctTotal)
    }

    pub fn label(self) -> String {
        match self {
            Metric::Count => "Count".into(),
            Metric::PctParent => "% of parent".into(),
            Metric::PctTotal => "% of total".into(),
            Metric::Median => "Median".into(),
            Metric::Mean => "Mean".into(),
            Metric::GeoMean => "Geo. mean".into(),
            Metric::Mode => "Mode".into(),
            Metric::Sd => "SD".into(),
            Metric::Cv => "CV %".into(),
            Metric::RobustCv => "rCV %".into(),
            Metric::Mad => "MAD".into(),
            Metric::Min => "Min".into(),
            Metric::Max => "Max".into(),
            Metric::Percentile(p) => format!("P{p}"),
        }
    }

    pub const COMMON: [Metric; 13] = [
        Metric::Count,
        Metric::PctParent,
        Metric::PctTotal,
        Metric::Median,
        Metric::Mean,
        Metric::GeoMean,
        Metric::Mode,
        Metric::Sd,
        Metric::Cv,
        Metric::RobustCv,
        Metric::Mad,
        Metric::Min,
        Metric::Max,
    ];
}

#[derive(Debug, Clone)]
pub struct Row {
    pub sample: String,
    /// Population path, e.g. `["Lymphocytes", "CD3+"]` (empty = All Events).
    pub population: Vec<String>,
    /// One value per requested metric; `None` = not defined (empty population, missing channel...).
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub metrics: Vec<Metric>,
    pub channel: Option<String>,
    pub rows: Vec<Row>,
    /// Things the reader must know to interpret the numbers (geometric-mean exclusions, missing channels).
    pub notes: Vec<String>,
}

pub fn pop_label(path: &[String]) -> String {
    if path.is_empty() {
        "All Events".into()
    } else {
        path.join(" ▸ ")
    }
}

/// Compute the table for `samples` (all populations of each tree, parents before children).
pub fn compute(session: &mut Session, samples: &[usize], metrics: &[Metric], channel: Option<&str>) -> Table {
    let mut rows = Vec::new();
    let mut notes = Vec::new();
    for &si in samples {
        let sname = session.sample(si).name.clone();
        let nodes = session.tree(si).self_and_descendants(GateTree::ROOT).unwrap_or_default();
        let total = session.sample(si).n_events;
        let col = channel.and_then(|c| session.sample(si).channel_index(c));
        if channel.is_some() && col.is_none() && metrics.iter().any(|m| m.needs_channel()) {
            notes.push(format!("{sname}: channel '{}' is not in this sample; channel statistics are blank", channel.unwrap()));
        }
        // evaluate once; collect masks' rows per node
        let Ok(ev) = session.evaluation(si) else { continue };
        let counts = ev.counts.clone();
        let rows_of: Vec<Option<Vec<usize>>> = ev.masks.iter().map(|m| m.as_ref().map(|m| m.indices())).collect();
        let parents: Vec<Option<usize>> = nodes.iter().map(|&n| session.tree(si).parent(n).ok().flatten()).collect();
        let s = session.sample(si);
        let d = s.channels.len();
        let mut geo_excluded = 0usize;
        for (k, &node) in nodes.iter().enumerate() {
            let path = session.tree(si).path(node).unwrap_or_default();
            let values: Vec<f64> = match (col, &rows_of[node]) {
                (Some(c), Some(r)) => r.iter().map(|&i| s.events[i * d + c] as f64).collect(),
                _ => Vec::new(),
            };
            let have = !values.is_empty();
            let out: Vec<Option<f64>> = metrics
                .iter()
                .map(|m| match m {
                    Metric::Count => Some(counts[node] as f64),
                    Metric::PctParent => {
                        let denom = parents[k].map(|p| counts[p]).unwrap_or(total);
                        (denom > 0).then(|| 100.0 * counts[node] as f64 / denom as f64)
                    }
                    Metric::PctTotal => (total > 0).then(|| 100.0 * counts[node] as f64 / total as f64),
                    _ if !have => None,
                    Metric::Median => Some(stats::median(&values)),
                    Metric::Mean => Some(stats::mean(&values)),
                    Metric::GeoMean => {
                        let g = stats::geo_mean(&values);
                        geo_excluded = geo_excluded.max(g.excluded);
                        (g.excluded < g.total).then_some(g.value)
                    }
                    Metric::Mode => Some(stats::mode(&values)),
                    Metric::Sd => Some(stats::sd(&values)),
                    Metric::Cv => Some(stats::cv(&values)),
                    Metric::RobustCv => Some(stats::rcv(&values)),
                    Metric::Mad => Some(stats::mad(&values)),
                    Metric::Min => Some(stats::min(&values)),
                    Metric::Max => Some(stats::max(&values)),
                    Metric::Percentile(p) => Some(stats::percentile(&values, *p)),
                })
                .map(|v| v.filter(|x| x.is_finite()))
                .collect();
            if metrics.contains(&Metric::GeoMean) && have {
                let g = stats::geo_mean(&values);
                if g.excluded > 0 {
                    notes.push(format!("{sname} / {}: geometric mean ignored {} of {} events with value <= 0", pop_label(&path), g.excluded, g.total));
                }
            }
            rows.push(Row { sample: sname.clone(), population: path, values: out });
        }
        let _ = geo_excluded;
    }
    Table { metrics: metrics.to_vec(), channel: channel.map(str::to_string), rows, notes }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// CSV with a header row; blanks for undefined values; full float precision (no display rounding).
pub fn to_csv(t: &Table) -> String {
    let mut out = String::from("Sample,Population");
    for m in &t.metrics {
        let label = match (&t.channel, m.needs_channel()) {
            (Some(c), true) => format!("{} ({c})", m.label()),
            _ => m.label(),
        };
        out.push(',');
        out.push_str(&csv_field(&label));
    }
    out.push('\n');
    for r in &t.rows {
        out.push_str(&csv_field(&r.sample));
        out.push(',');
        out.push_str(&csv_field(&pop_label(&r.population)));
        for v in &r.values {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&format!("{v}"));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::sample::LoadedSample;

    /// 10 events: x = 0..9 (FITC), y = 1 (PE).
    fn session() -> (Session, usize) {
        let events: Vec<f32> = (0..10).flat_map(|i| [i as f32, 1.0]).collect();
        let mut s = Session::new();
        let i = s.add_sample(LoadedSample::synthetic("a.fcs", &["FITC-A", "PE-A"], vec![1024.0; 2], events));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        // "High" = FITC in [5, 10): 5 events (5..9)
        {
            let win = s.window_mut(w).unwrap();
            win.x_scale = sf_core::scale::ScaleSpec::Identity;
            win.y_scale = sf_core::scale::ScaleSpec::Identity;
        }
        s.commit_shape(w, &Shape::Rect { x0: 5.0, x1: 10.0, y0: -1.0, y1: 5.0 }, "High").unwrap();
        (s, i)
    }

    fn val(t: &Table, pop: &str, m: Metric) -> Option<f64> {
        let k = t.metrics.iter().position(|x| *x == m).unwrap();
        t.rows.iter().find(|r| pop_label(&r.population) == pop).unwrap().values[k]
    }

    #[test]
    fn counts_percentages_and_channel_statistics_match_hand_values() {
        let (mut s, i) = session();
        let t = compute(&mut s, &[i], &[Metric::Count, Metric::PctParent, Metric::PctTotal, Metric::Median, Metric::Mean, Metric::Sd, Metric::Min, Metric::Max, Metric::Percentile(50.0)], Some("FITC-A"));
        assert_eq!(t.rows.len(), 2);
        assert_eq!((val(&t, "All Events", Metric::Count), val(&t, "High", Metric::Count)), (Some(10.0), Some(5.0)));
        assert_eq!(val(&t, "High", Metric::PctParent), Some(50.0));
        assert_eq!(val(&t, "All Events", Metric::PctParent), Some(100.0));
        assert_eq!(val(&t, "High", Metric::PctTotal), Some(50.0));
        // High = {5,6,7,8,9}
        assert_eq!(val(&t, "High", Metric::Median), Some(7.0));
        assert_eq!(val(&t, "High", Metric::Mean), Some(7.0));
        assert!((val(&t, "High", Metric::Sd).unwrap() - 2.0f64.sqrt()).abs() < 1e-12, "population SD, not sample SD");
        assert_eq!((val(&t, "High", Metric::Min), val(&t, "High", Metric::Max)), (Some(5.0), Some(9.0)));
        assert_eq!(val(&t, "All Events", Metric::Percentile(50.0)), Some(4.5), "type-7 percentile of 0..9");
    }

    #[test]
    fn geometric_mean_reports_what_it_dropped() {
        let (mut s, i) = session();
        let t = compute(&mut s, &[i], &[Metric::GeoMean], Some("FITC-A"));
        // All Events contains one zero: dropped and said so
        assert!(t.notes.iter().any(|n| n.contains("All Events") && n.contains("1 of 10")), "{:?}", t.notes);
        let all = val(&t, "All Events", Metric::GeoMean).unwrap();
        let want = ((1..10).map(|x| (x as f64).ln()).sum::<f64>() / 9.0).exp();
        assert!((all - want).abs() < 1e-9);
        assert!(!t.notes.iter().any(|n| n.contains("High")), "High has no non-positive values: {:?}", t.notes);
    }

    #[test]
    fn a_missing_channel_blanks_channel_statistics_but_keeps_counts() {
        let (mut s, i) = session();
        let t = compute(&mut s, &[i], &[Metric::Count, Metric::Median], Some("BV421-A"));
        assert_eq!(val(&t, "High", Metric::Count), Some(5.0));
        assert_eq!(val(&t, "High", Metric::Median), None);
        assert!(t.notes.iter().any(|n| n.contains("BV421-A")));
        let none = compute(&mut s, &[i], &[Metric::Count], None);
        assert!(none.notes.is_empty());
    }

    #[test]
    fn an_empty_population_has_counts_but_no_channel_statistics() {
        let (mut s, i) = session();
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        s.commit_shape(w, &Shape::Rect { x0: 100.0, x1: 200.0, y0: -1.0, y1: 5.0 }, "Nothing").unwrap();
        let t = compute(&mut s, &[i], &[Metric::Count, Metric::PctParent, Metric::Mean], Some("FITC-A"));
        assert_eq!(val(&t, "Nothing", Metric::Count), Some(0.0));
        assert_eq!(val(&t, "Nothing", Metric::PctParent), Some(0.0));
        assert_eq!(val(&t, "Nothing", Metric::Mean), None);
    }

    #[test]
    fn csv_has_a_header_escapes_fields_and_leaves_blanks() {
        let (mut s, i) = session();
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        s.commit_shape(w, &Shape::Rect { x0: 100.0, x1: 200.0, y0: -1.0, y1: 5.0 }, "Has, comma").unwrap();
        let t = compute(&mut s, &[i], &[Metric::Count, Metric::Mean], Some("FITC-A"));
        let csv = to_csv(&t);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "Sample,Population,Count,Mean (FITC-A)");
        assert_eq!(lines[1], "a.fcs,All Events,10,4.5");
        assert!(lines.iter().any(|l| l.starts_with("a.fcs,\"Has, comma\",0,")) && lines.iter().any(|l| l.ends_with(",0,")), "{csv}");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn statistics_follow_compensated_channels() {
        let (mut s, i) = session();
        let info = crate::sample::CompInfo { detectors: vec!["FITC-A".into(), "PE-A".into()], spill: vec![1.0, 0.0, 0.0, 1.0], source: "t".into() };
        s.set_compensation(i, &info, sf_core::compensation::Mode::Standard).unwrap();
        let t = compute(&mut s, &[i], &[Metric::Mean], Some("Comp-FITC-A"));
        assert_eq!(val(&t, "All Events", Metric::Mean), Some(4.5));
    }
}
