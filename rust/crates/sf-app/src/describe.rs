//! "What is this population?": the markers that set a population apart from the rest of its parent. This is the
//! step from a region drawn on a map (or any gate) to a name. Pure numbers; nothing is random.

use crate::session::Session;
use sf_core::stats;
use sf_gating::NodeId;

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerDiff {
    pub channel: String,
    /// Median inside the population and in the rest of the parent (raw units).
    pub inside: f64,
    pub outside: f64,
    /// Chance that an event of the population is higher on this marker than an event outside it (0..1).
    /// 0.5 = no difference, near 1 = clearly higher (positive), near 0 = clearly lower.
    pub auc: f64,
    /// Difference of the medians on the arcsinh scale, in robust standard deviations of the outside events.
    pub effect: f64,
}

impl MarkerDiff {
    /// `+`, `-` or nothing: only clear differences get a sign.
    pub fn call(&self) -> &'static str {
        if self.auc >= 0.85 {
            "+"
        } else if self.auc <= 0.15 {
            "-"
        } else {
            ""
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Description {
    pub n_in: usize,
    pub n_out: usize,
    /// Markers, the most distinguishing first.
    pub markers: Vec<MarkerDiff>,
}

/// Chance that a value of `a` exceeds a value of `b` (ties count half): the area under the ROC curve.
pub fn auc(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.5;
    }
    let mut all: Vec<(f64, bool)> = a.iter().map(|&v| (v, true)).chain(b.iter().map(|&v| (v, false))).collect();
    all.sort_by(|x, y| x.0.total_cmp(&y.0));
    // sum of the (tie-averaged) ranks of `a`
    let (mut rank_sum, mut i) = (0.0, 0usize);
    while i < all.len() {
        let mut j = i;
        while j + 1 < all.len() && all[j + 1].0 == all[i].0 {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        rank_sum += avg * all[i..=j].iter().filter(|e| e.1).count() as f64;
        i = j + 1;
    }
    let (na, nb) = (a.len() as f64, b.len() as f64);
    (rank_sum - na * (na + 1.0) / 2.0) / (na * nb)
}

/// Compare a population with the rest of its parent on `channels`. At most 4000 events of each side are used
/// (an even stride), which is plenty for medians and ranks and keeps it instant on large files.
pub fn describe(session: &mut Session, sample: usize, node: NodeId, channels: &[String], cofactor: f64) -> Result<Description, String> {
    let parent = session.tree(sample).parent(node).map_err(|e| e.to_string())?.ok_or("All Events has nothing to be compared with: choose a population")?;
    let n_events = session.sample(sample).n_events;
    let (inside, of_parent) = {
        let ev = session.evaluation(sample)?;
        let rows = |n: NodeId| -> Result<Vec<usize>, String> { Ok(ev.masks.get(n).and_then(|m| m.as_ref()).ok_or("population has no events")?.indices()) };
        (rows(node)?, rows(parent)?)
    };
    let mut is_in = vec![false; n_events];
    for &r in &inside {
        is_in[r] = true;
    }
    let outside: Vec<usize> = of_parent.into_iter().filter(|&r| !is_in[r]).collect();
    if inside.len() < 20 || outside.len() < 20 {
        return Err(format!("too few events to compare ({} inside, {} in the rest of the parent)", inside.len(), outside.len()));
    }
    let thin = |v: &[usize]| -> Vec<usize> {
        let step = (v.len() / 4000).max(1);
        v.iter().copied().step_by(step).collect()
    };
    let (ri, ro) = (thin(&inside), thin(&outside));
    let s = session.sample(sample);
    let d = s.channels.len();
    let tf = |v: f64| if cofactor > 0.0 { (v / cofactor).asinh() } else { v };
    let mut markers = Vec::new();
    for ch in channels {
        let Some(c) = s.channel_index(ch) else { continue };
        let take = |r: &[usize]| -> Vec<f64> { r.iter().map(|&r| s.events[r * d + c] as f64).filter(|v| v.is_finite()).collect() };
        let (a, b) = (take(&ri), take(&ro));
        if a.len() < 20 || b.len() < 20 {
            continue;
        }
        let (ta, tb): (Vec<f64>, Vec<f64>) = (a.iter().map(|&v| tf(v)).collect(), b.iter().map(|&v| tf(v)).collect());
        let spread = (1.4826 * stats::mad(&tb)).max(1e-9);
        markers.push(MarkerDiff { channel: ch.clone(), inside: stats::median(&a), outside: stats::median(&b), auc: auc(&ta, &tb), effect: (stats::median(&ta) - stats::median(&tb)) / spread });
    }
    markers.sort_by(|x, y| (y.auc - 0.5).abs().total_cmp(&(x.auc - 0.5).abs()));
    Ok(Description { n_in: inside.len(), n_out: outside.len(), markers })
}

/// A name from the clear differences: up to four `+` markers, then up to two `-` ones (`CD4+ CD25+ CD8-`).
/// Empty when nothing sets the population apart.
pub fn suggest_name(d: &Description, label: &dyn Fn(&str) -> String) -> String {
    let pos = d.markers.iter().filter(|m| m.call() == "+").take(4).map(|m| format!("{}+", label(&m.channel)));
    let neg = d.markers.iter().filter(|m| m.call() == "-").take(2).map(|m| format!("{}-", label(&m.channel)));
    pos.chain(neg).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auc_is_the_chance_of_being_higher_with_ties_counted_half() {
        assert_eq!(auc(&[3.0, 4.0], &[1.0, 2.0]), 1.0);
        assert_eq!(auc(&[1.0, 2.0], &[3.0, 4.0]), 0.0);
        assert_eq!(auc(&[1.0, 1.0], &[1.0, 1.0]), 0.5);
        assert_eq!(auc(&[1.0, 3.0], &[2.0]), 0.5);
        assert_eq!(auc(&[], &[1.0]), 0.5);
        // 2 of 4 pairs higher, 1 tie
        assert!((auc(&[2.0, 5.0], &[2.0, 4.0]) - (1.0 + 0.5 + 1.0) / 4.0).abs() < 1e-12);
    }
}
