//! A display/gating scale as plain data: what an axis (and a gate drawn on it) is expressed in.
//!
//! A gate drawn on a logicle axis lives in logicle space, so the gate must carry the scale to be tested
//! correctly; this is that description. It round-trips through a short text form (`arcsinh:150`,
//! `logicle:262144,0.5,4.5,0`, ...) used in saved workspaces.

use crate::flowjo::{Biex, WspLog};
use crate::logicle::Logicle;
use crate::transforms::{Arcsinh, Transform};

#[derive(Debug, Clone, PartialEq)]
pub enum ScaleSpec {
    /// Raw data values.
    Identity,
    /// `(x - min) / (max - min)`: FlowJo's linear axis.
    Linear { min: f64, max: f64 },
    /// `asinh(x / cofactor)` (the engine's arcsinh).
    Arcsinh { cofactor: f64 },
    Log { offset: f64, decades: f64 },
    Biex { neg: f64, width: f64, pos: f64, max: f64 },
    Logicle { t: f64, w: f64, m: f64, a: f64 },
}

/// `(x - min) / (max - min)`.
#[derive(Debug, Clone, Copy)]
struct LinearScale {
    min: f64,
    span: f64,
}

impl Transform for LinearScale {
    fn forward(&self, x: f64) -> f64 {
        (x - self.min) / self.span
    }
    fn inverse(&self, y: f64) -> f64 {
        y * self.span + self.min
    }
    fn name(&self) -> &'static str {
        "linear"
    }
}

#[derive(Debug, Clone, Copy)]
struct IdentityScale;

impl Transform for IdentityScale {
    fn forward(&self, x: f64) -> f64 {
        x
    }
    fn inverse(&self, y: f64) -> f64 {
        y
    }
    fn name(&self) -> &'static str {
        "identity"
    }
}

impl ScaleSpec {
    pub fn is_identity(&self) -> bool {
        matches!(self, ScaleSpec::Identity)
    }

    pub fn build(&self) -> Result<Box<dyn Transform + Send + Sync>, String> {
        Ok(match *self {
            ScaleSpec::Identity => Box::new(IdentityScale),
            ScaleSpec::Linear { min, max } => {
                if !(max > min) || !min.is_finite() || !max.is_finite() {
                    return Err(format!("linear axis needs max > min (got {min}..{max})"));
                }
                Box::new(LinearScale { min, span: max - min })
            }
            ScaleSpec::Arcsinh { cofactor } => {
                if !(cofactor > 0.0) || !cofactor.is_finite() {
                    return Err(format!("arcsinh cofactor must be positive (got {cofactor})"));
                }
                Box::new(Arcsinh::new(cofactor))
            }
            ScaleSpec::Log { offset, decades } => Box::new(WspLog::new(offset, decades)?),
            ScaleSpec::Biex { neg, width, pos, max } => Box::new(Biex::new(neg, width, pos, max)?),
            ScaleSpec::Logicle { t, w, m, a } => Box::new(Logicle::new(t, w, m, a)?),
        })
    }

    /// Short text form, the inverse of [`ScaleSpec::parse`].
    pub fn to_text(&self) -> String {
        match *self {
            ScaleSpec::Identity => "linear".into(),
            ScaleSpec::Linear { min, max } => format!("range:{min},{max}"),
            ScaleSpec::Arcsinh { cofactor } => format!("arcsinh:{cofactor}"),
            ScaleSpec::Log { offset, decades } => format!("log:{offset},{decades}"),
            ScaleSpec::Biex { neg, width, pos, max } => format!("biex:{neg},{width},{pos},{max}"),
            ScaleSpec::Logicle { t, w, m, a } => format!("logicle:{t},{w},{m},{a}"),
        }
    }

    /// Parse the text form. Bare names written by the Java app (`Linear`, `Arcsinh`, `Logicle`, `Log`) are
    /// accepted with `None` for the parameters it did not store: `Ok((spec_with_defaults, true))` flags
    /// that defaults were assumed so the caller can say so.
    pub fn parse(s: &str) -> Result<(ScaleSpec, bool), String> {
        let (name, args) = match s.split_once(':') {
            Some((n, a)) => (n.trim().to_ascii_lowercase(), Some(a)),
            None => (s.trim().to_ascii_lowercase(), None),
        };
        let nums = |a: &str, want: usize| -> Result<Vec<f64>, String> {
            let v: Vec<f64> = a.split(',').map(|t| t.trim().parse::<f64>().map_err(|_| format!("bad number '{t}' in scale '{s}'"))).collect::<Result<_, _>>()?;
            if v.len() != want {
                return Err(format!("scale '{s}' needs {want} numbers"));
            }
            Ok(v)
        };
        Ok(match (name.as_str(), args) {
            ("linear" | "none" | "identity", None) => (ScaleSpec::Identity, false),
            ("range", Some(a)) => {
                let v = nums(a, 2)?;
                (ScaleSpec::Linear { min: v[0], max: v[1] }, false)
            }
            ("arcsinh", Some(a)) => (ScaleSpec::Arcsinh { cofactor: nums(a, 1)?[0] }, false),
            ("arcsinh", None) => (ScaleSpec::Arcsinh { cofactor: 150.0 }, true), // the engine's default cofactor
            ("log", Some(a)) => {
                let v = nums(a, 2)?;
                (ScaleSpec::Log { offset: v[0], decades: v[1] }, false)
            }
            ("biex", Some(a)) => {
                let v = nums(a, 4)?;
                (ScaleSpec::Biex { neg: v[0], width: v[1], pos: v[2], max: v[3] }, false)
            }
            ("logicle", Some(a)) => {
                let v = nums(a, 4)?;
                (ScaleSpec::Logicle { t: v[0], w: v[1], m: v[2], a: v[3] }, false)
            }
            ("logicle", None) => (ScaleSpec::Logicle { t: 262144.0, w: 0.5, m: 4.5, a: 0.0 }, true), // FlowJo/flowutils defaults
            _ => return Err(format!("unknown scale '{s}'")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_round_trips_through_text_and_builds_a_working_transform() {
        let specs = [
            ScaleSpec::Identity,
            ScaleSpec::Linear { min: 0.0, max: 262144.0 },
            ScaleSpec::Arcsinh { cofactor: 5.0 },
            ScaleSpec::Log { offset: 1.0, decades: 4.0 },
            ScaleSpec::Biex { neg: 0.0, width: -10.0, pos: 4.41854, max: 262144.0 },
            ScaleSpec::Logicle { t: 262144.0, w: 0.5, m: 4.5, a: 0.0 },
        ];
        for s in specs {
            let (back, assumed) = ScaleSpec::parse(&s.to_text()).unwrap();
            assert_eq!((&back, assumed), (&s, false), "{}", s.to_text());
            let t = s.build().unwrap();
            let y = t.forward(1234.5);
            assert!(y.is_finite(), "{s:?}");
            assert!((t.inverse(y) - 1234.5).abs() < 1e-3 * 1234.5, "{s:?}: {}", t.inverse(y));
        }
    }

    #[test]
    fn java_bare_names_are_accepted_but_flag_assumed_parameters() {
        assert_eq!(ScaleSpec::parse("Linear").unwrap(), (ScaleSpec::Identity, false));
        assert_eq!(ScaleSpec::parse("Arcsinh").unwrap(), (ScaleSpec::Arcsinh { cofactor: 150.0 }, true));
        assert!(matches!(ScaleSpec::parse("Logicle").unwrap(), (ScaleSpec::Logicle { .. }, true)));
        assert!(ScaleSpec::parse("sqrt").is_err() && ScaleSpec::parse("arcsinh:abc").is_err() && ScaleSpec::parse("logicle:1,2").is_err());
        assert!(ScaleSpec::Linear { min: 5.0, max: 5.0 }.build().is_err());
        assert!(ScaleSpec::Arcsinh { cofactor: 0.0 }.build().is_err());
    }
}
