//! Minimal strict JSON (RFC 8259) — enough for workspaces and the engine protocol, no dependencies.
//!
//! Numbers are `f64`; non-finite numbers cannot be written (JSON has no Infinity/NaN) and are rejected
//! by [`Json::write`] callers via [`Json::num`], which maps them to `null`.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Keys are kept sorted so output is deterministic (stable diffs of saved workspaces).
    Obj(BTreeMap<String, Json>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for JsonError {}

/// Parser nesting limit: deep input must produce an error, never a stack overflow.
const MAX_DEPTH: usize = 256;

impl Json {
    /// A number, or `null` when `v` is NaN/infinite (which JSON cannot represent).
    pub fn num(v: f64) -> Json {
        if v.is_finite() {
            Json::Num(v)
        } else {
            Json::Null
        }
    }

    pub fn obj() -> Json {
        Json::Obj(BTreeMap::new())
    }

    /// Builder: `Json::obj().with("a", 1)`.
    pub fn with(mut self, key: &str, v: Json) -> Json {
        if let Json::Obj(m) = &mut self {
            m.insert(key.to_string(), v);
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Result<Json, JsonError> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return p.err("trailing characters after the document");
        }
        Ok(v)
    }

    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.is_finite() {
                    // Rust's shortest round-trip representation; integral values print without a fraction.
                    out.push_str(&format!("{n}"));
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => write_str(s, out),
            Json::Arr(a) => {
                out.push('[');
                for (k, v) in a.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    v.write_into(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                for (k, (key, v)) in m.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    write_str(key, out);
                    out.push(':');
                    v.write_into(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn err<T>(&self, m: &str) -> Result<T, JsonError> {
        Err(JsonError { offset: self.i, message: m.to_string() })
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str, v: Json) -> Result<Json, JsonError> {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            Ok(v)
        } else {
            self.err("unexpected token")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return self.err("nesting is too deep");
        }
        match self.s.get(self.i) {
            None => self.err("unexpected end of input"),
            Some(b'n') => self.eat("null", Json::Null),
            Some(b't') => self.eat("true", Json::Bool(true)),
            Some(b'f') => self.eat("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("expected a string key");
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expected ':'");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.i;
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        let digits = |p: &mut Parser| {
            let b = p.i;
            while p.i < p.s.len() && p.s[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - b
        };
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1, // no leading zeros
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return self.err("expected a digit"),
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return self.err("expected digits after '.'");
            }
        }
        if matches!(self.s.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return self.err("expected exponent digits");
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).expect("ascii");
        match text.parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Json::Num(v)),
            _ => Err(JsonError { offset: start, message: format!("number {text} is out of range") }),
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.i + 4 > self.s.len() {
            return self.err("truncated \\u escape");
        }
        let t = std::str::from_utf8(&self.s[self.i..self.i + 4]).map_err(|_| JsonError { offset: self.i, message: "bad \\u escape".into() })?;
        let v = u32::from_str_radix(t, 16).map_err(|_| JsonError { offset: self.i, message: "bad \\u escape".into() })?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let run = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' && self.s[self.i] >= 0x20 {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[run..self.i]).map_err(|_| JsonError { offset: run, message: "invalid UTF-8".into() })?);
            match self.s.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = match self.s.get(self.i) {
                        Some(c) => *c,
                        None => return self.err("unterminated escape"),
                    };
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.s.get(self.i) == Some(&b'\\') && self.s.get(self.i + 1) == Some(&b'u') {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return self.err("unpaired surrogate");
                                    }
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                } else {
                                    return self.err("unpaired surrogate");
                                }
                            }
                            match char::from_u32(cp) {
                                Some(ch) => out.push(ch),
                                None => return self.err("invalid code point"),
                            }
                        }
                        _ => return self.err("unknown escape"),
                    }
                }
                Some(_) => return self.err("control character in string"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_a_document() {
        let v = Json::parse(r#" {"a":[1,2.5,-3e2,true,null],"b":"x\ny\u00e9\ud83d\ude00","c":{}} "#).unwrap();
        assert_eq!(v.get("a").unwrap().as_arr().unwrap()[2].as_f64(), Some(-300.0));
        assert_eq!(v.get("b").unwrap().as_str(), Some("x\nyé😀"));
        let again = Json::parse(&v.write()).unwrap();
        assert_eq!(v, again);
    }

    #[test]
    fn numbers_round_trip_bit_exactly() {
        for x in [0.1, 1.0 / 3.0, 1e-300, 1.7976931348623157e308, -0.0, 123456789.123456789, 5e-324] {
            let t = Json::Num(x).write();
            let back = Json::parse(&t).unwrap().as_f64().unwrap();
            assert_eq!(back.to_bits(), x.to_bits(), "{x} -> {t}");
        }
    }

    #[test]
    fn non_finite_numbers_become_null_not_invalid_json() {
        assert_eq!(Json::num(f64::NAN), Json::Null);
        assert_eq!(Json::Num(f64::INFINITY).write(), "null");
    }

    #[test]
    fn rejects_malformed_input_with_a_position() {
        for bad in ["", "{", "[1,]", "{\"a\":}", "01", "1.", "\"abc", "[1] x", "{'a':1}", "\"\\x\"", "1e999", "\"\u{1}\"", "\"\\ud800\""] {
            assert!(Json::parse(bad).is_err(), "should reject {bad:?}");
        }
        let e = Json::parse("[1, 2,, 3]").unwrap_err();
        assert_eq!(e.offset, 6, "the second comma of `[1, 2,, 3]` is byte 6");
    }

    #[test]
    fn absurd_nesting_is_an_error_not_a_stack_overflow() {
        let s = "[".repeat(100_000);
        assert!(Json::parse(&s).is_err());
    }

    #[test]
    fn output_is_deterministic_and_escapes_control_characters() {
        let v = Json::obj().with("z", Json::Num(1.0)).with("a", Json::Str("q\"\u{1}".into()));
        assert_eq!(v.write(), "{\"a\":\"q\\\"\\u0001\",\"z\":1}");
    }
}
