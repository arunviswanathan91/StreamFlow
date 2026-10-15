//! A loaded FCS sample, including FlowJo-style compensation: compensated detectors appear as extra
//! channels named `Comp-<detector>` appended after the file's own channels, so axes, gates and statistics
//! treat them like any other channel and nothing downstream needs a special case.

use sf_core::compensation::{Compensation, Mode};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const COMP_PREFIX: &str = "Comp-";
pub const UNMIX_PREFIX: &str = "Unmix-";

/// The spillover matrix applied to a sample.
#[derive(Debug, Clone, PartialEq)]
pub struct CompInfo {
    pub detectors: Vec<String>,
    /// Row `i` is the spillover of detector `i` into every detector (row-major `n x n`).
    pub spill: Vec<f64>,
    /// Where it came from ("embedded $SPILLOVER", "edited", ...), for display.
    pub source: String,
}

impl CompInfo {
    pub fn identity(detectors: Vec<String>) -> CompInfo {
        let n = detectors.len();
        let mut spill = vec![0.0; n * n];
        for i in 0..n {
            spill[i * n + i] = 1.0;
        }
        CompInfo { detectors, spill, source: "identity".into() }
    }

    pub fn is_identity(&self) -> bool {
        let n = self.detectors.len();
        (0..n * n).all(|q| self.spill[q] == if q / n == q % n { 1.0 } else { 0.0 })
    }
}

/// Parse an FCS `$SPILLOVER` / `$SPILL` value: `n,name1,...,namen,v11,...,vnn` (row-major).
pub fn parse_spillover(text: &str) -> Result<(Vec<String>, Vec<f64>), String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    let n: usize = parts.first().and_then(|p| p.parse().ok()).ok_or("spillover does not start with a channel count")?;
    if n == 0 || parts.len() != 1 + n + n * n {
        return Err(format!("spillover declares {n} channels but has {} fields (expected {})", parts.len(), 1 + n + n * n));
    }
    let names: Vec<String> = parts[1..=n].iter().map(|s| s.to_string()).collect();
    let vals: Vec<f64> = parts[n + 1..].iter().map(|v| v.parse::<f64>().map_err(|_| format!("bad spillover value '{v}'"))).collect::<Result<_, _>>()?;
    if vals.iter().any(|v| !v.is_finite()) {
        return Err("spillover contains a non-finite value".into());
    }
    Ok((names, vals))
}

#[derive(Debug, Clone)]
pub struct LoadedSample {
    pub name: String,
    pub path: Option<PathBuf>,
    pub channels: Vec<String>,
    /// `$PnS` marker labels (may be empty strings).
    pub labels: Vec<String>,
    pub ranges: Vec<f64>,
    pub events: Vec<f32>,
    pub n_events: usize,
    /// FCS TEXT keywords (upper-cased keys).
    pub keywords: HashMap<String, String>,
    /// Number of channels that came from the file; anything after is a derived `Comp-` channel.
    pub n_base: usize,
    pub comp: Option<CompInfo>,
    /// True while `Unmix-` channels from spectral unmixing are present.
    pub unmixed: bool,
}

impl LoadedSample {
    pub fn from_fcs(path: &Path) -> Result<LoadedSample, String> {
        let f = sf_fcs::Fcs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let n_base = f.params.len();
        Ok(LoadedSample {
            name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            path: Some(path.to_path_buf()),
            channels: f.params.iter().map(|p| p.name.clone()).collect(),
            labels: f.params.iter().map(|p| p.stain.clone()).collect(),
            ranges: f.params.iter().map(|p| p.range).collect(),
            n_events: f.n_events,
            events: f.events,
            keywords: f.text,
            n_base,
            comp: None,
            unmixed: false,
        })
    }

    /// A sample built from in-memory data (tests, synthetic data).
    pub fn synthetic(name: &str, channels: &[&str], ranges: Vec<f64>, events: Vec<f32>) -> LoadedSample {
        let n = channels.len();
        LoadedSample {
            name: name.into(),
            path: None,
            labels: vec![String::new(); n],
            channels: channels.iter().map(|s| s.to_string()).collect(),
            ranges,
            n_events: if n == 0 { 0 } else { events.len() / n },
            events,
            keywords: HashMap::new(),
            n_base: n,
            comp: None,
            unmixed: false,
        }
    }

    pub fn channel_index(&self, name: &str) -> Option<usize> {
        self.channels.iter().position(|c| c == name)
    }

    pub fn range_of(&self, name: &str) -> f64 {
        self.channel_index(name).map_or(262_144.0, |i| self.ranges[i])
    }

    /// The matrix embedded in the file, if any.
    pub fn embedded_spillover(&self) -> Option<Result<CompInfo, String>> {
        // keys differ between writers (`$SPILLOVER`, `$SPILL`, `SPILL`, ...); match on the bare name
        let v = self.keywords.iter().find(|(k, _)| matches!(k.trim().trim_start_matches('$'), "SPILLOVER" | "SPILL")).map(|(_, v)| v)?;
        Some(parse_spillover(v).map(|(detectors, spill)| CompInfo { detectors, spill, source: "embedded $SPILLOVER".into() }))
    }

    /// Fluorescence detectors that are not already derived `Comp-` channels.
    pub fn base_channels(&self) -> &[String] {
        &self.channels[..self.n_base]
    }

    /// Remove derived columns (index >= `n_base`) whose name starts with `prefix`; the file's own columns and
    /// derived columns of other kinds are untouched.
    fn drop_derived(&mut self, prefix: &str) {
        let keep: Vec<usize> = (0..self.channels.len()).filter(|&c| c < self.n_base || !self.channels[c].starts_with(prefix)).collect();
        if keep.len() == self.channels.len() {
            return;
        }
        let d = self.channels.len();
        let mut out = Vec::with_capacity(self.n_events * keep.len());
        for row in self.events.chunks_exact(d) {
            out.extend(keep.iter().map(|&c| row[c]));
        }
        self.events = out;
        self.channels = keep.iter().map(|&c| self.channels[c].clone()).collect();
        self.labels = keep.iter().map(|&c| self.labels[c].clone()).collect();
        self.ranges = keep.iter().map(|&c| self.ranges[c]).collect();
    }

    /// Append derived channels; `cols[c][event]` holds channel `c`'s values.
    fn append_derived(&mut self, names: Vec<String>, labels: Vec<String>, ranges: Vec<f64>, cols: &[Vec<f32>]) {
        let (d_old, k) = (self.channels.len(), names.len());
        let mut out = Vec::with_capacity(self.n_events * (d_old + k));
        for (r, row) in self.events.chunks_exact(d_old).enumerate() {
            out.extend_from_slice(row);
            out.extend(cols.iter().map(|c| c[r]));
        }
        self.events = out;
        self.channels.extend(names);
        self.labels.extend(labels);
        self.ranges.extend(ranges);
    }

    /// Remove the compensated (`Comp-`) channels only.
    pub fn clear_compensation(&mut self) {
        self.drop_derived(COMP_PREFIX);
        self.comp = None;
    }

    /// Compensate with `info` and append the results as `Comp-<detector>` channels (replacing any earlier
    /// compensation). Every detector must exist in the file; the original columns are never modified, and a
    /// failure changes nothing.
    pub fn set_compensation(&mut self, info: &CompInfo, mode: Mode) -> Result<(), String> {
        let cols: Vec<usize> = info
            .detectors
            .iter()
            .map(|d| self.channels[..self.n_base].iter().position(|c| c == d).ok_or_else(|| format!("detector '{d}' of the matrix is not a channel of {}", self.name)))
            .collect::<Result<_, _>>()?;
        let comp = Compensation::new(info.detectors.clone(), info.spill.clone()).map_err(|e| e.to_string())?;
        let d = self.channels.len();
        let mut work = self.events.clone();
        comp.apply(&mut work, d, &cols, mode).map_err(|e| e.to_string())?;
        let data: Vec<Vec<f32>> = cols.iter().map(|&c| (0..self.n_events).map(|r| work[r * d + c]).collect()).collect();
        let names: Vec<String> = info.detectors.iter().map(|det| format!("{COMP_PREFIX}{det}")).collect();
        let labels: Vec<String> = cols.iter().map(|&c| self.labels[c].clone()).collect();
        let ranges: Vec<f64> = cols.iter().map(|&c| self.ranges[c]).collect();
        self.clear_compensation();
        self.append_derived(names, labels, ranges, &data);
        self.comp = Some(info.clone());
        Ok(())
    }

    /// Replace the unmixed (`Unmix-`) channels with these columns (`data[c][event]`).
    pub fn set_unmixed(&mut self, names: Vec<String>, data: Vec<Vec<f32>>, range: f64) -> Result<(), String> {
        if names.len() != data.len() || data.iter().any(|c| c.len() != self.n_events) {
            return Err("unmixed columns do not match the sample's event count".into());
        }
        let names: Vec<String> = names.into_iter().map(|n| format!("{UNMIX_PREFIX}{n}")).collect();
        self.drop_derived(UNMIX_PREFIX);
        let k = names.len();
        self.append_derived(names, vec![String::new(); k], vec![range; k], &data);
        self.unmixed = true;
        Ok(())
    }

    pub fn clear_unmixed(&mut self) {
        self.drop_derived(UNMIX_PREFIX);
        self.unmixed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_color() -> LoadedSample {
        // FITC and PE with 30% of FITC spilling into PE: PE_measured = PE_true + 0.3 * FITC_true
        let fitc = [1000.0f32, 0.0, 500.0];
        let pe = [300.0f32, 2000.0, 150.0];
        let events: Vec<f32> = (0..3).flat_map(|i| [fitc[i], pe[i], 7.0]).collect();
        LoadedSample::synthetic("s", &["FITC-A", "PE-A", "FSC-A"], vec![262144.0; 3], events)
    }

    fn matrix() -> CompInfo {
        CompInfo { detectors: vec!["FITC-A".into(), "PE-A".into()], spill: vec![1.0, 0.3, 0.0, 1.0], source: "test".into() }
    }

    #[test]
    fn spillover_text_parses_and_bad_text_is_rejected() {
        let (n, v) = parse_spillover("2,FITC-A,PE-A,1,0.3,0,1").unwrap();
        assert_eq!((n, v), (vec!["FITC-A".to_string(), "PE-A".to_string()], vec![1.0, 0.3, 0.0, 1.0]));
        for bad in ["", "x,a,b", "2,a,b,1,0,0", "2,a,b,1,0,0,1,9", "0", "1,a,nan"] {
            assert!(parse_spillover(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn compensation_appends_comp_channels_with_the_correct_numbers_and_leaves_the_originals() {
        let mut s = two_color();
        let before = s.events.clone();
        s.set_compensation(&matrix(), Mode::Standard).unwrap();
        assert_eq!(s.channels, vec!["FITC-A", "PE-A", "FSC-A", "Comp-FITC-A", "Comp-PE-A"]);
        assert_eq!((s.ranges.len(), s.labels.len()), (5, 5));
        let d = 5;
        for (i, (fitc, pe)) in [(1000.0f32, 300.0f32), (0.0, 2000.0), (500.0, 150.0)].iter().enumerate() {
            let row = &s.events[i * d..(i + 1) * d];
            assert_eq!((row[0], row[1], row[2]), (*fitc, *pe, 7.0), "original columns untouched");
            // true values: FITC unchanged, PE = measured - 0.3 * FITC
            assert!((row[3] - fitc).abs() < 1e-3, "{} vs {fitc}", row[3]);
            assert!((row[4] - (pe - 0.3 * fitc)).abs() < 1e-3, "{} vs {}", row[4], pe - 0.3 * fitc);
        }
        // the first event was pure spillover: all of its PE signal is FITC
        assert!(s.events[4].abs() < 1e-3);
        // removing restores the original matrix exactly
        s.clear_compensation();
        assert_eq!(s.events, before);
        assert!(s.comp.is_none() && s.channels.len() == 3);
    }

    #[test]
    fn recompensating_replaces_rather_than_stacks_and_bad_matrices_change_nothing() {
        let mut s = two_color();
        s.set_compensation(&matrix(), Mode::Standard).unwrap();
        s.set_compensation(&CompInfo::identity(vec!["FITC-A".into(), "PE-A".into()]), Mode::Standard).unwrap();
        assert_eq!(s.channels.len(), 5, "no duplicate Comp- channels");
        assert!(s.events[4] - s.events[1] == 0.0, "identity leaves values unchanged");
        let snapshot = (s.channels.clone(), s.events.clone());
        let missing = CompInfo { detectors: vec!["FITC-A".into(), "BV421-A".into()], spill: vec![1.0, 0.0, 0.0, 1.0], source: "x".into() };
        assert!(s.set_compensation(&missing, Mode::Standard).unwrap_err().contains("BV421-A"));
        let singular = CompInfo { detectors: vec!["FITC-A".into(), "PE-A".into()], spill: vec![1.0, 1.0, 1.0, 1.0], source: "x".into() };
        assert!(s.set_compensation(&singular, Mode::Standard).is_err());
        assert_eq!((s.channels, s.events), snapshot, "a failed compensation must leave the sample as it was");
        assert!(CompInfo::identity(vec!["a".into(), "b".into()]).is_identity() && !matrix().is_identity());
    }

    #[test]
    fn the_real_fixtures_carry_an_embedded_matrix() {
        let p = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain/AF700 alone.fcs"));
        if !p.exists() {
            eprintln!("SKIPPED: fixture missing");
            return;
        }
        let mut s = LoadedSample::from_fcs(p).unwrap();
        let info = s.embedded_spillover().expect("AF700 alone.fcs has $SPILLOVER").unwrap();
        assert_eq!(info.detectors.len(), 6);
        assert!(info.is_identity(), "this file ships an identity matrix");
        s.set_compensation(&info, Mode::Standard).unwrap();
        assert_eq!(s.channels.len(), s.n_base + 6);
        let k = s.channel_index("Comp-FITC-A").unwrap();
        let o = s.channel_index("FITC-A").unwrap();
        let d = s.channels.len();
        assert!(s.events.chunks(d).take(1000).all(|r| (r[k] - r[o]).abs() < 1e-3), "identity matrix changes nothing");
    }
}

/// Parse a spillover matrix from CSV/TSV text: a header row of detector names (an empty first cell is
/// allowed) then one row per detector, optionally starting with its name. Values may be fractions
/// (diagonal 1) or percentages (diagonal 100); the diagonal decides.
pub fn parse_matrix_csv(text: &str) -> Result<CompInfo, String> {
    let sep = if text.contains('\t') { '\t' } else { ',' };
    let rows: Vec<Vec<&str>> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.split(sep).map(str::trim).collect()).collect();
    if rows.len() < 2 {
        return Err("a matrix file needs a header row and at least one data row".into());
    }
    let header: Vec<String> = rows[0].iter().filter(|c| !c.is_empty()).map(|c| c.trim_matches('"').to_string()).collect();
    let n = header.len();
    if n == 0 || rows.len() - 1 != n {
        return Err(format!("expected {n} data rows for {n} detectors, found {}", rows.len() - 1));
    }
    let mut vals = Vec::with_capacity(n * n);
    for (i, r) in rows[1..].iter().enumerate() {
        let cells: Vec<&str> = if r.len() == n + 1 { r[1..].to_vec() } else { r.clone() };
        if cells.len() != n {
            return Err(format!("row {} has {} values, expected {n}", i + 1, cells.len()));
        }
        if r.len() == n + 1 && r[0].trim_matches('"') != header[i] {
            return Err(format!("row {} is labelled '{}' but the header says '{}'", i + 1, r[0], header[i]));
        }
        for c in cells {
            let v: f64 = c.trim_matches('"').trim_end_matches('%').parse().map_err(|_| format!("'{c}' is not a number"))?;
            if !v.is_finite() {
                return Err(format!("'{c}' is not finite"));
            }
            vals.push(v);
        }
    }
    let percent = (0..n).map(|i| vals[i * n + i]).all(|d| d > 50.0);
    if percent {
        vals.iter_mut().for_each(|v| *v /= 100.0);
    }
    if (0..n).any(|i| (vals[i * n + i] - 1.0).abs() > 1e-6) {
        return Err("the diagonal of a spillover matrix must be 1 (or 100%)".into());
    }
    Ok(CompInfo { detectors: header, spill: vals, source: "imported matrix".into() })
}

/// CSV with a header row and a name column, values as fractions.
pub fn matrix_to_csv(m: &CompInfo) -> String {
    let n = m.detectors.len();
    let mut out = format!(",{}\n", m.detectors.join(","));
    for i in 0..n {
        out.push_str(&m.detectors[i]);
        for j in 0..n {
            out.push_str(&format!(",{}", m.spill[i * n + j]));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod csv_tests {
    use super::*;

    #[test]
    fn matrices_round_trip_through_csv_and_percent_files_are_normalised() {
        let m = CompInfo { detectors: vec!["FITC-A".into(), "PE-A".into(), "APC-A".into()], spill: vec![1.0, 0.12, 0.0, 0.03, 1.0, 0.2, 0.0, 0.05, 1.0], source: "x".into() };
        let back = parse_matrix_csv(&matrix_to_csv(&m)).unwrap();
        assert_eq!((back.detectors.clone(), back.spill.clone()), (m.detectors.clone(), m.spill.clone()));
        let pct = "FITC-A\tPE-A\n100\t12\n3\t100\n";
        let p = parse_matrix_csv(pct).unwrap();
        assert_eq!(p.spill, vec![1.0, 0.12, 0.03, 1.0]);
        // quoted names and a % suffix are tolerated
        let q = parse_matrix_csv("\"A\",\"B\"\n100%,5%\n0%,100%\n").unwrap();
        assert_eq!(q.spill, vec![1.0, 0.05, 0.0, 1.0]);
    }

    #[test]
    fn malformed_matrices_are_refused_with_a_reason() {
        for (bad, why) in [
            ("A,B\n1,0\n", "data rows"),
            ("A,B\n1,0\n0\n", "values"),
            ("A,B\nA,1,0\nC,0,1\n", "labelled"),
            ("A,B\n2,0\n0,1\n", "diagonal"),
            ("A,B\n1,x\n0,1\n", "not a number"),
            ("", "header"),
        ] {
            let e = parse_matrix_csv(bad).unwrap_err();
            assert!(e.contains(why), "{bad:?}: {e}");
        }
    }
}
