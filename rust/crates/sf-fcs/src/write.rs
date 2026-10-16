//! FCS 3.1 writer (list mode, 32-bit float, little-endian): enough to save a population as a new file that this
//! reader and other cytometry software open.

use crate::Param;

/// Keywords the writer sets itself; any of these in `extra` are ignored.
const OWN: &[&str] = &["$BEGINDATA", "$ENDDATA", "$BEGINANALYSIS", "$ENDANALYSIS", "$BEGINSTEXT", "$ENDSTEXT", "$NEXTDATA", "$BYTEORD", "$DATATYPE", "$MODE", "$PAR", "$TOT"];

fn escape(v: &str) -> String {
    v.replace('/', "//")
}

/// `events` is row-major `n x params.len()`. `extra` keywords (instrument, date, ...) are kept; per-parameter
/// keywords are rebuilt from `params`.
pub fn to_bytes(params: &[Param], events: &[f32], extra: &[(String, String)]) -> Result<Vec<u8>, String> {
    let p = params.len();
    if p == 0 || events.len() % p != 0 {
        return Err("The event matrix does not match the number of parameters".into());
    }
    let n = events.len() / p;
    let data_len = events.len() * 4;
    // TEXT first with placeholder offsets of fixed width, so its length is known before the offsets are
    let mk_text = |data_start: usize| -> String {
        let mut kv: Vec<(String, String)> = vec![
            ("$BEGINANALYSIS".into(), "0".into()),
            ("$ENDANALYSIS".into(), "0".into()),
            ("$BEGINSTEXT".into(), "0".into()),
            ("$ENDSTEXT".into(), "0".into()),
            ("$BEGINDATA".into(), format!("{data_start:010}")),
            ("$ENDDATA".into(), format!("{:010}", data_start + data_len.max(1) - 1)),
            ("$BYTEORD".into(), "1,2,3,4".into()),
            ("$DATATYPE".into(), "F".into()),
            ("$MODE".into(), "L".into()),
            ("$NEXTDATA".into(), "0".into()),
            ("$PAR".into(), p.to_string()),
            ("$TOT".into(), n.to_string()),
        ];
        for (i, pa) in params.iter().enumerate() {
            let i = i + 1;
            kv.push((format!("$P{i}B"), "32".into()));
            kv.push((format!("$P{i}E"), "0,0".into()));
            kv.push((format!("$P{i}N"), pa.name.clone()));
            kv.push((format!("$P{i}R"), format!("{}", pa.range.max(1.0).round() as u64)));
            if !pa.stain.is_empty() {
                kv.push((format!("$P{i}S"), pa.stain.clone()));
            }
        }
        for (k, v) in extra {
            let k = k.trim().to_uppercase();
            let per_param = k.starts_with("$P") && k[2..].chars().next().map_or(false, |c| c.is_ascii_digit());
            if !k.is_empty() && !OWN.contains(&k.as_str()) && !per_param && !v.is_empty() {
                kv.push((k, v.clone()));
            }
        }
        let mut t = String::from("/");
        for (k, v) in kv {
            t.push_str(&escape(&k));
            t.push('/');
            t.push_str(&escape(&v));
            t.push('/');
        }
        t
    };
    let text_start = 58;
    let len = mk_text(0).len();
    let text_end = text_start + len - 1;
    let data_start = text_end + 1;
    let text = mk_text(data_start);
    debug_assert_eq!(text.len(), len);
    let data_end = data_start + data_len.max(1) - 1;
    let header_data = if data_end > 99_999_999 { (0, 0) } else { (data_start, data_end) };
    let mut out = format!("FCS3.1    {text_start:>8}{text_end:>8}{:>8}{:>8}{:>8}{:>8}", header_data.0, header_data.1, 0, 0).into_bytes();
    out.resize(text_start, b' ');
    out.extend_from_slice(text.as_bytes());
    for v in events {
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Fcs;

    fn params() -> Vec<Param> {
        ["FSC-A", "FITC/A", "SSC-A"].iter().map(|n| Param { name: n.to_string(), stain: if *n == "FITC/A" { "CD3".into() } else { String::new() }, range: 262144.0, bits: 32, exponent: "0,0".into() }).collect()
    }

    #[test]
    fn what_is_written_reads_back_exactly() {
        let ev: Vec<f32> = (0..30).map(|i| i as f32 * 1.5 - 7.0).collect();
        let bytes = to_bytes(&params(), &ev, &[("$CYT".into(), "Test/Instrument".into()), ("$P1N".into(), "ignored".into()), ("$TOT".into(), "99".into())]).unwrap();
        let f = Fcs::parse(&bytes).unwrap();
        assert_eq!((f.n_events, f.n_params()), (10, 3));
        assert_eq!(f.events, ev);
        assert_eq!(f.params[1].name, "FITC/A", "a slash in a name survives escaping");
        assert_eq!(f.params[1].stain, "CD3");
        assert_eq!(f.params[0].range, 262144.0);
        assert_eq!(f.text.get("$CYT").map(String::as_str), Some("Test/Instrument"));
        assert_eq!(f.text.get("$TOT").map(String::as_str), Some("10"), "the writer's own count wins");
        assert_eq!(f.params[0].name, "FSC-A");
    }

    #[test]
    fn an_empty_population_and_a_bad_matrix_are_handled() {
        let f = Fcs::parse(&to_bytes(&params(), &[], &[]).unwrap());
        assert!(f.map_or(true, |f| f.n_events == 0));
        assert!(to_bytes(&params(), &[1.0, 2.0], &[]).is_err());
        assert!(to_bytes(&[], &[], &[]).is_err());
    }
}
