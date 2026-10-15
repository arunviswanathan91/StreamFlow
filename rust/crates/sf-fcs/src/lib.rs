//! FCS 2.0 / 3.0 / 3.1 reader (list mode only).
//!
//! Events are returned **raw**: no `$PnE` log decoding and no `$PnG` gain, matching the engine's
//! `source="raw"`. Integer data are masked to the parameter's `$PnR` range when that range is a
//! power of two, as the specification requires. Errors carry a real message; nothing falls back
//! silently.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub enum FcsError {
    Io(std::io::Error),
    Format(String),
}

impl fmt::Display for FcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FcsError::Io(e) => write!(f, "I/O error: {e}"),
            FcsError::Format(m) => write!(f, "invalid FCS file: {m}"),
        }
    }
}

impl std::error::Error for FcsError {}

impl From<std::io::Error> for FcsError {
    fn from(e: std::io::Error) -> Self {
        FcsError::Io(e)
    }
}

fn bad<T>(m: impl Into<String>) -> Result<T, FcsError> {
    Err(FcsError::Format(m.into()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// `$PnN` short name (the channel name, e.g. `FITC-A`).
    pub name: String,
    /// `$PnS` stain / marker label; empty when absent.
    pub stain: String,
    /// `$PnR` range.
    pub range: f64,
    /// `$PnB` bits.
    pub bits: u32,
    /// `$PnE` as written (`"0,0"` = linear).
    pub exponent: String,
}

#[derive(Debug, Clone)]
pub struct Fcs {
    pub version: String,
    /// TEXT keywords, keys upper-cased (`$TOT`, `$P1N`, ...).
    pub text: HashMap<String, String>,
    pub params: Vec<Param>,
    pub n_events: usize,
    /// Row-major `n_events x params.len()`.
    pub events: Vec<f32>,
}

impl Fcs {
    pub fn read(path: impl AsRef<Path>) -> Result<Fcs, FcsError> {
        Fcs::parse(&fs::read(path)?)
    }

    pub fn n_params(&self) -> usize {
        self.params.len()
    }

    pub fn channel_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    /// One channel as `f64`, or `None` for an unknown name.
    pub fn column(&self, name: &str) -> Option<Vec<f64>> {
        let c = self.channel_index(name)?;
        let p = self.params.len();
        Some((0..self.n_events).map(|r| self.events[r * p + c] as f64).collect())
    }

    pub fn parse(b: &[u8]) -> Result<Fcs, FcsError> {
        if b.len() < 58 || &b[0..3] != b"FCS" {
            return bad("missing FCS header");
        }
        let version = String::from_utf8_lossy(&b[0..6]).trim().to_string();
        let field = |i: usize| -> usize {
            // 8-character, space-padded ASCII integers
            let s = String::from_utf8_lossy(&b[10 + i * 8..18 + i * 8]);
            s.trim().parse::<usize>().unwrap_or(0)
        };
        let (t0, t1, mut d0, mut d1) = (field(0), field(1), field(2), field(3));
        if t0 == 0 || t1 < t0 || t1 >= b.len() {
            return bad(format!("bad TEXT offsets {t0}..{t1} for a {}-byte file", b.len()));
        }

        let text = parse_text(&b[t0..=t1])?;
        // Large files: the header offsets are 0 and the real ones live in TEXT.
        if d0 == 0 && d1 == 0 {
            d0 = kw_usize(&text, "$BEGINDATA").unwrap_or(0);
            d1 = kw_usize(&text, "$ENDDATA").unwrap_or(0);
        }

        let mode = text.get("$MODE").map(|s| s.trim().to_uppercase()).unwrap_or_else(|| "L".into());
        if mode != "L" {
            return bad(format!("unsupported $MODE '{mode}' (only list mode)"));
        }
        let n_params = kw_usize(&text, "$PAR").ok_or_else(|| FcsError::Format("missing $PAR".into()))?;
        let n_events = kw_usize(&text, "$TOT").ok_or_else(|| FcsError::Format("missing $TOT".into()))?;
        let dtype = text.get("$DATATYPE").map(|s| s.trim().to_uppercase()).unwrap_or_default();

        let mut params = Vec::with_capacity(n_params);
        for i in 1..=n_params {
            let key = |s: &str| format!("$P{i}{s}");
            params.push(Param {
                name: text.get(&key("N")).cloned().unwrap_or_else(|| format!("P{i}")),
                stain: text.get(&key("S")).cloned().unwrap_or_default(),
                range: text.get(&key("R")).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0),
                bits: kw_usize(&text, &key("B")).unwrap_or(0) as u32,
                exponent: text.get(&key("E")).cloned().unwrap_or_else(|| "0,0".into()),
            });
        }

        let big = match text.get("$BYTEORD").map(|s| s.trim()) {
            Some("4,3,2,1") | Some("2,1") => true,
            Some("1,2,3,4") | Some("1,2") | None => false,
            Some(o) => return bad(format!("unsupported $BYTEORD '{o}'")),
        };

        if d1 < d0 || d1 >= b.len() {
            return bad(format!("bad DATA offsets {d0}..{d1} for a {}-byte file", b.len()));
        }
        let data = &b[d0..=d1];
        let total = n_events * n_params;
        let mut events = Vec::with_capacity(total);

        match dtype.as_str() {
            "F" => read_floats::<4>(data, total, big, &mut events, |a| f32::from_le_bytes(a))?,
            "D" => read_floats::<8>(data, total, big, &mut events, |a| f64::from_le_bytes(a) as f32)?,
            "I" => read_ints(data, &params, n_events, big, &mut events)?,
            other => return bad(format!("unsupported $DATATYPE '{other}'")),
        }
        Ok(Fcs { version, text, params, n_events, events })
    }
}

/// Fixed-width floats; each N-byte chunk is put in little-endian order before `conv` decodes it.
fn read_floats<const N: usize>(
    data: &[u8],
    count: usize,
    big: bool,
    out: &mut Vec<f32>,
    conv: impl Fn([u8; N]) -> f32,
) -> Result<(), FcsError> {
    if data.len() < count * N {
        return bad(format!("DATA segment holds {} bytes, need {}", data.len(), count * N));
    }
    for c in data[..count * N].chunks_exact(N) {
        let mut a = [0u8; N];
        a.copy_from_slice(c);
        if big {
            a.reverse();
        }
        out.push(conv(a));
    }
    Ok(())
}

fn read_ints(data: &[u8], params: &[Param], n_events: usize, big: bool, out: &mut Vec<f32>) -> Result<(), FcsError> {
    let widths: Vec<usize> = params.iter().map(|p| (p.bits as usize).div_ceil(8)).collect();
    if widths.iter().any(|w| ![1, 2, 4].contains(w)) {
        return bad("integer data needs $PnB of 8, 16 or 32 bits for every parameter");
    }
    let row: usize = widths.iter().sum();
    if data.len() < row * n_events {
        return bad(format!("DATA segment holds {} bytes, need {}", data.len(), row * n_events));
    }
    // mask = range-1 when the range is a power of two (the spec: unused high bits are not data)
    let masks: Vec<u64> = params
        .iter()
        .map(|p| {
            let r = p.range as u64;
            if r > 1 && r.is_power_of_two() {
                r - 1
            } else {
                u64::MAX
            }
        })
        .collect();
    let mut pos = 0;
    for _ in 0..n_events {
        for (w, m) in widths.iter().zip(&masks) {
            let mut v: u64 = 0;
            for k in 0..*w {
                let byte = data[pos + if big { k } else { w - 1 - k }] as u64;
                v = (v << 8) | byte;
            }
            pos += w;
            out.push((v & m) as f32);
        }
    }
    Ok(())
}

fn kw_usize(text: &HashMap<String, String>, key: &str) -> Option<usize> {
    text.get(key).and_then(|s| s.trim().parse::<usize>().ok())
}

/// TEXT is `D key D value D key D value D`; a doubled delimiter inside a token is a literal one.
fn parse_text(seg: &[u8]) -> Result<HashMap<String, String>, FcsError> {
    let Some((&delim, rest)) = seg.split_first() else {
        return bad("empty TEXT segment");
    };
    let mut tokens: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == delim {
            if i + 1 < rest.len() && rest[i + 1] == delim {
                cur.push(delim);
                i += 2;
                continue;
            }
            tokens.push(std::mem::take(&mut cur));
        } else {
            cur.push(rest[i]);
        }
        i += 1;
    }
    if !cur.is_empty() {
        tokens.push(cur); // writers that omit the closing delimiter
    }
    let mut map = HashMap::new();
    for pair in tokens.chunks(2) {
        if let [k, v] = pair {
            map.insert(String::from_utf8_lossy(k).trim().to_uppercase(), String::from_utf8_lossy(v).into_owned());
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid FCS 3.1 file in memory.
    fn make(dtype: &str, byteord: &str, bits: u32, range: u64, data: Vec<u8>, par: usize, tot: usize) -> Vec<u8> {
        let d = b'/';
        let mut t: Vec<u8> = vec![d];
        let mut kv = |k: &str, v: &str| {
            t.extend(k.as_bytes());
            t.push(d);
            t.extend(v.as_bytes());
            t.push(d);
        };
        kv("$BYTEORD", byteord);
        kv("$DATATYPE", dtype);
        kv("$MODE", "L");
        kv("$PAR", &par.to_string());
        kv("$TOT", &tot.to_string());
        for p in 1..=par {
            kv(&format!("$P{p}N"), &format!("CH{p}"));
            kv(&format!("$P{p}S"), &format!("marker{p}"));
            kv(&format!("$P{p}B"), &bits.to_string());
            kv(&format!("$P{p}R"), &range.to_string());
            kv(&format!("$P{p}E"), "0,0");
        }
        let t0 = 256usize;
        let t1 = t0 + t.len() - 1;
        let d0 = t1 + 1;
        let d1 = d0 + data.len() - 1;
        let mut out = format!("FCS3.1    {t0:8}{t1:8}{d0:8}{d1:8}{:8}{:8}", 0, 0).into_bytes();
        out.resize(t0, b' ');
        out.extend(t);
        out.extend(data);
        out
    }

    #[test]
    fn reads_float32_little_endian() {
        let vals: [f32; 6] = [1.0, 2.5, -3.0, 4.0, 5.0, 6.25];
        let data: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = Fcs::parse(&make("F", "1,2,3,4", 32, 262144, data, 2, 3)).unwrap();
        assert_eq!((f.n_events, f.n_params()), (3, 2));
        assert_eq!(f.events, vals);
        assert_eq!(f.params[1].name, "CH2");
        assert_eq!(f.params[1].stain, "marker2");
        assert_eq!(f.column("CH1").unwrap(), vec![1.0, -3.0, 5.0]);
        assert!(f.column("nope").is_none());
    }

    #[test]
    fn reads_float32_big_endian() {
        let vals: [f32; 2] = [7.5, -0.125];
        let data: Vec<u8> = vals.iter().flat_map(|v| v.to_be_bytes()).collect();
        let f = Fcs::parse(&make("F", "4,3,2,1", 32, 1024, data, 1, 2)).unwrap();
        assert_eq!(f.events, vals);
    }

    #[test]
    fn reads_float64() {
        let data: Vec<u8> = [1.5f64, 2.5].iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = Fcs::parse(&make("D", "1,2,3,4", 64, 1024, data, 1, 2)).unwrap();
        assert_eq!(f.events, vec![1.5, 2.5]);
    }

    #[test]
    fn integer_data_is_masked_to_the_power_of_two_range() {
        // range 1024 -> 10 data bits; 0xFC05 & 0x3FF = 5
        let data: Vec<u8> = vec![0x05, 0xFC, 0x10, 0x00];
        let f = Fcs::parse(&make("I", "1,2", 16, 1024, data, 1, 2)).unwrap();
        assert_eq!(f.events, vec![5.0, 16.0]);
    }

    #[test]
    fn rejects_garbage_and_truncation_with_a_message() {
        assert!(matches!(Fcs::parse(b"nope"), Err(FcsError::Format(m)) if m.contains("header")));
        let mut f = make("F", "1,2,3,4", 32, 1024, vec![0u8; 8], 1, 2);
        f.truncate(f.len() - 4);
        assert!(Fcs::parse(&f).is_err());
        let lying = make("F", "1,2,3,4", 32, 1024, vec![0u8; 4], 1, 5); // $TOT lies
        assert!(Fcs::parse(&lying).unwrap_err().to_string().contains("DATA"));
    }

    #[test]
    fn doubled_delimiter_is_a_literal() {
        let m = parse_text(b"/$A/x//y/$B/z/").unwrap();
        assert_eq!(m["$A"], "x/y");
        assert_eq!(m["$B"], "z");
    }
}
