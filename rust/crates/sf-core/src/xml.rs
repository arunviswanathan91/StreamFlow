//! A small, safe XML reader for workspaces and GatingML.
//!
//! Scope: elements, attributes, text, comments, CDATA, processing instructions, the five predefined
//! entities and numeric character references. Namespaces are reduced to local names (`gating:min` ->
//! `min`), which is what workspace formats need. DOCTYPE declarations are skipped and **never
//! expanded**, so entity-expansion attacks are impossible; nesting is capped; parsing is iterative.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Local name (namespace prefix removed).
    pub name: String,
    /// `(local name, value)` in document order; `xmlns` declarations are dropped.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// Concatenated character data directly inside this element.
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Every descendant (not including self), document order, without recursion.
    pub fn descendants(&self) -> Vec<&Element> {
        let (mut out, mut stack) = (Vec::new(), vec![self]);
        while let Some(e) = stack.pop() {
            for c in e.children.iter().rev() {
                stack.push(c);
            }
            if !std::ptr::eq(e, self) {
                out.push(e);
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmlError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid XML at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for XmlError {}

const MAX_DEPTH: usize = 2000;

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub fn parse(text: &str) -> Result<Element, XmlError> {
    let s = text.strip_prefix('\u{feff}').unwrap_or(text);
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let err = |i: usize, m: &str| XmlError { offset: i, message: m.to_string() };

    while i < b.len() {
        if b[i] != b'<' {
            let end = s[i..].find('<').map_or(b.len(), |p| i + p);
            let raw = &s[i..end];
            match stack.last_mut() {
                Some(top) => top.text.push_str(&unescape(raw).map_err(|m| err(i, &m))?),
                None if raw.trim().is_empty() => {}
                None => return Err(err(i, "text outside the root element")),
            }
            i = end;
            continue;
        }
        if s[i..].starts_with("<!--") {
            i = s[i + 4..].find("-->").map(|p| i + 4 + p + 3).ok_or_else(|| err(i, "unterminated comment"))?;
        } else if s[i..].starts_with("<![CDATA[") {
            let end = s[i..].find("]]>").map(|p| i + p).ok_or_else(|| err(i, "unterminated CDATA"))?;
            match stack.last_mut() {
                Some(top) => top.text.push_str(&s[i + 9..end]),
                None => return Err(err(i, "CDATA outside the root element")),
            }
            i = end + 3;
        } else if s[i..].starts_with("<?") {
            i = s[i..].find("?>").map(|p| i + p + 2).ok_or_else(|| err(i, "unterminated processing instruction"))?;
        } else if s[i..].starts_with("<!") {
            // DOCTYPE: skip to the matching '>' (honouring one level of [...] internal subset); never expanded
            let mut depth = 0i32;
            let mut j = i + 2;
            while j < b.len() {
                match b[j] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => break,
                    _ => {}
                }
                j += 1;
            }
            if j >= b.len() {
                return Err(err(i, "unterminated declaration"));
            }
            i = j + 1;
        } else if s[i..].starts_with("</") {
            let end = s[i..].find('>').map(|p| i + p).ok_or_else(|| err(i, "unterminated end tag"))?;
            let name = local(s[i + 2..end].trim());
            let done = stack.pop().ok_or_else(|| err(i, "end tag with no open element"))?;
            if done.name != name {
                return Err(err(i, &format!("end tag </{name}> does not match <{}>", done.name)));
            }
            i = end + 1;
            match stack.last_mut() {
                Some(parent) => parent.children.push(done),
                None => {
                    if root.is_some() {
                        return Err(err(i, "more than one root element"));
                    }
                    root = Some(done);
                }
            }
        } else {
            // start tag
            let (el, self_closing, next) = start_tag(s, i).map_err(|(o, m)| err(o, &m))?;
            i = next;
            if self_closing {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => {
                        if root.is_some() {
                            return Err(err(i, "more than one root element"));
                        }
                        root = Some(el);
                    }
                }
            } else {
                if stack.len() >= MAX_DEPTH {
                    return Err(err(i, "nesting is too deep"));
                }
                stack.push(el);
            }
        }
    }
    if let Some(open) = stack.last() {
        return Err(err(b.len(), &format!("<{}> is never closed", open.name)));
    }
    root.ok_or_else(|| err(0, "no root element"))
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.') || c >= 0x80
}

/// Parse `<name a="v" ...>` or `<name .../>` starting at `start`; returns the element, whether it
/// self-closes, and the index after `>`.
fn start_tag(s: &str, start: usize) -> Result<(Element, bool, usize), (usize, String)> {
    let b = s.as_bytes();
    let mut i = start + 1;
    let ns = i;
    while i < b.len() && is_name_char(b[i]) {
        i += 1;
    }
    if i == ns {
        return Err((start, "expected an element name".into()));
    }
    let mut el = Element { name: local(&s[ns..i]).to_string(), ..Default::default() };
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        match b.get(i) {
            None => return Err((i, "unterminated start tag".into())),
            Some(b'>') => return Ok((el, false, i + 1)),
            Some(b'/') if b.get(i + 1) == Some(&b'>') => return Ok((el, true, i + 2)),
            Some(_) => {
                let an = i;
                while i < b.len() && is_name_char(b[i]) {
                    i += 1;
                }
                if i == an {
                    return Err((i, "expected an attribute name".into()));
                }
                let key = &s[an..i];
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                if b.get(i) != Some(&b'=') {
                    return Err((i, format!("attribute '{key}' has no value")));
                }
                i += 1;
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                let q = match b.get(i) {
                    Some(q @ (b'"' | b'\'')) => *q,
                    _ => return Err((i, "attribute value must be quoted".into())),
                };
                let vs = i + 1;
                let ve = s[vs..].find(q as char).map(|p| vs + p).ok_or((i, "unterminated attribute value".to_string()))?;
                let val = unescape(&s[vs..ve]).map_err(|m| (vs, m))?;
                i = ve + 1;
                if key == "xmlns" || key.starts_with("xmlns:") {
                    continue;
                }
                let lk = local(key).to_string();
                if el.attrs.iter().any(|(k, _)| *k == lk) {
                    // prefixed duplicates (a:min / b:min) would silently shadow each other
                    return Err((an, format!("duplicate attribute '{lk}'")));
                }
                el.attrs.push((lk, val));
            }
        }
    }
}

fn unescape(raw: &str) -> Result<String, String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let semi = after.find(';').ok_or("unterminated entity reference")?;
        let ent = &after[..semi];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                let cp = u32::from_str_radix(&ent[2..], 16).map_err(|_| format!("bad character reference &{ent};"))?;
                out.push(char::from_u32(cp).ok_or_else(|| format!("invalid code point in &{ent};"))?);
            }
            _ if ent.starts_with('#') => {
                let cp: u32 = ent[1..].parse().map_err(|_| format!("bad character reference &{ent};"))?;
                out.push(char::from_u32(cp).ok_or_else(|| format!("invalid code point in &{ent};"))?);
            }
            _ => return Err(format!("unknown entity &{ent}; (custom entities are not supported)")),
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Escape text for an attribute value or element content.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaces_attributes_text_and_entities() {
        let e = parse(
            "<?xml version=\"1.0\"?><!-- c --><a:Root xmlns:a=\"u\" id='1'><a:K name=\"x&amp;y\" v=\"&#65;&#x42;\"/>t&lt;1<![CDATA[<raw>]]><b/></a:Root>",
        )
        .unwrap();
        assert_eq!(e.name, "Root");
        assert_eq!(e.attr("id"), Some("1"));
        assert_eq!(e.attrs.len(), 1, "xmlns declarations are dropped");
        let k = e.child("K").unwrap();
        assert_eq!((k.attr("name"), k.attr("v")), (Some("x&y"), Some("AB")));
        assert_eq!(e.text, "t<1<raw>");
        assert_eq!(e.children.len(), 2);
    }

    #[test]
    fn prefixed_attributes_use_their_local_name() {
        let e = parse("<g:G xmlns:g=\"u\" xmlns:d=\"v\"><g:dimension g:min=\"1\" g:max=\"2\"><d:fcs-dimension d:name=\"FSC-A\"/></g:dimension></g:G>").unwrap();
        let d = e.child("dimension").unwrap();
        assert_eq!((d.attr("min"), d.attr("max")), (Some("1"), Some("2")));
        assert_eq!(d.child("fcs-dimension").unwrap().attr("name"), Some("FSC-A"));
        assert_eq!(e.descendants().len(), 2);
    }

    #[test]
    fn malformed_documents_are_errors_with_positions() {
        for bad in ["", "<a>", "<a></b>", "<a b=1/>", "<a b=\"1\" b=\"2\"/>", "<a/><b/>", "x<a/>", "<a>&nope;</a>", "<a b=\"", "<a><!-- x </a>", "</a>", "<a>&#xD800;</a>"] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
        assert_eq!(parse("<a></b>").unwrap_err().offset, 3);
    }

    #[test]
    fn doctype_entities_are_never_expanded_and_deep_nesting_is_an_error() {
        let e = parse("<!DOCTYPE a [<!ENTITY x \"boom\">]><a>&amp;</a>").unwrap();
        assert_eq!(e.text, "&");
        assert!(parse("<!DOCTYPE a [<!ENTITY x \"boom\">]><a>&x;</a>").is_err());
        let deep = "<a>".repeat(100_000);
        assert!(parse(&deep).is_err());
        let ok = format!("{}{}", "<a>".repeat(1500), "</a>".repeat(1500));
        assert_eq!(parse(&ok).unwrap().descendants().len(), 1499);
    }

    #[test]
    fn escape_round_trips_through_the_parser() {
        let raw = "a<b>&\"c'";
        let e = parse(&format!("<a v=\"{}\">{}</a>", escape(raw), escape(raw))).unwrap();
        assert_eq!((e.attr("v"), e.text.as_str()), (Some(raw), raw));
    }
}
