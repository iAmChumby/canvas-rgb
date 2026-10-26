//! A lenient parser for SignalRGB-format effect files.
//!
//! SignalRGB effect files are small HTML documents that follow a fixed shape: a `<head>`
//! with a `<title>`, a `<meta description>` and one `<meta property=... label=... type=...>`
//! per property, a `<body>` with a `<canvas id=... width=... height=...>`, and one or more
//! inline `<script>` blocks that are the program.
//!
//! A full HTML5 parser would be overkill and would pull a large dependency tree into a
//! crate that must stay lean. This scanner only looks at the tags it needs and treats
//! everything else as text.

/// One parsed `<meta property=...>` element, with its attributes.
#[derive(Debug, Clone, Default)]
pub struct MetaTag {
    pub attrs: Vec<(String, String)>,
}

impl MetaTag {
    /// Attribute lookup is case-insensitive, as in HTML.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Everything the runtime needs from an effect file.
#[derive(Debug, Clone, Default)]
pub struct ParsedEffect {
    pub title: Option<String>,
    pub description: Option<String>,
    /// The property metas, in document order.
    pub metas: Vec<MetaTag>,
    /// Inline script bodies, in document order.
    pub scripts: Vec<String>,
    /// The canvas element's attributes (`id`, `width`, `height`).
    pub canvas: Option<MetaTag>,
}

/// Scans an effect file. Never fails: a file with no recognizable structure yields an
/// empty `ParsedEffect` and the caller reports a parse error.
pub fn parse(html: &str) -> ParsedEffect {
    let mut out = ParsedEffect::default();
    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        // Find the next '<'. Text outside tags is ignored.
        let Some(tag_start) = memchr(bytes, b'<', i) else { break };
        // Comments, doctypes and CDATA are skipped wholesale.
        if bytes[tag_start..].starts_with(b"<!--") {
            match find_sub(bytes, b"-->", tag_start + 4) {
                Some(end) => {
                    i = end + 3;
                    continue;
                }
                None => break,
            }
        }
        if bytes[tag_start..].starts_with(b"<!") {
            match memchr(bytes, b'>', tag_start) {
                Some(end) => {
                    i = end + 1;
                    continue;
                }
                None => break,
            }
        }
        // The end of the tag: '>' not inside a quoted attribute value.
        let Some(tag_end) = find_tag_end(bytes, tag_start) else { break };
        let raw = &html[tag_start + 1..tag_end];
        i = tag_end + 1;

        let Some(name) = tag_name(raw) else {
            // A closing tag or an unparsable one.
            continue;
        };
        let lower = name.to_ascii_lowercase();

        match lower.as_str() {
            "title" => {
                let text = read_text_until(html, i, "title");
                if out.title.is_none() && !text.trim().is_empty() {
                    out.title = Some(text.trim().to_string());
                }
            }
            "meta" => {
                let attrs = parse_attrs(raw);
                let tag = MetaTag { attrs };
                if tag.attr("property").is_some() {
                    out.metas.push(tag);
                } else if out.description.is_none() {
                    if let Some(d) = tag.attr("description") {
                        if !d.trim().is_empty() {
                            out.description = Some(d.trim().to_string());
                        }
                    }
                }
            }
            "canvas" => {
                if out.canvas.is_none() {
                    out.canvas = Some(MetaTag { attrs: parse_attrs(raw) });
                }
            }
            "script" => {
                // Inline script only: a `src` script has no body here.
                let attrs = parse_attrs(raw);
                let has_src = attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case("src"));
                let body = read_text_until(html, i, "script");
                if !has_src {
                    out.scripts.push(body);
                }
                i = skip_past_close(html, i, "script");
            }
            _ => {}
        }
    }

    out
}

/// The text content of the element that starts at `from`, up to its matching close tag.
/// The close tag is consumed; `from` points just after the opening tag.
fn read_text_until(html: &str, from: usize, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let close = format!("</{tag}");
    let Some(end) = lower[from..].find(&close) else {
        return html[from..].to_string();
    };
    html[from..from + end].to_string()
}

/// Advances `from` past the matching close tag of `tag`, if there is one.
fn skip_past_close(html: &str, from: usize, tag: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let close = format!("</{tag}");
    let Some(end) = lower[from..].find(&close) else {
        return html.len();
    };
    let after = from + end;
    match lower[after..].find('>') {
        Some(gt) => after + gt + 1,
        None => html.len(),
    }
}

/// The tag name of a raw tag body (without the angle brackets), if it is an opening tag.
fn tag_name(raw: &str) -> Option<&str> {
    let raw = raw.trim_start_matches('/');
    let name: String = raw
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(raw.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or(""))
    }
}

/// Parses the attributes of a tag body into (name, value) pairs. Values are unescaped
/// for the basic HTML entities an effect file can contain.
fn parse_attrs(raw: &str) -> Vec<(String, String)> {
    let bytes = raw.as_bytes();
    let mut attrs = Vec::new();
    let mut i = 0usize;

    // Skip the tag name.
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b':') {
        i += 1;
    }
    while i < bytes.len() {
        // Skip whitespace and any stray '/'.
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // The attribute name.
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
            && bytes[i] != b'>'
        {
            i += 1;
        }
        let name = raw[name_start..i].to_string();
        if name.is_empty() {
            i += 1;
            continue;
        }
        // Skip whitespace before '='.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = raw[value_start..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = raw[value_start..i].to_string();
            }
        }
        attrs.push((name, unescape(&value)));
    }
    attrs
}

/// The handful of HTML entities an effect file can contain in an attribute value.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// The index of the '>' that ends a tag, skipping quoted attribute values.
fn find_tag_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from + 1;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}

fn memchr(bytes: &[u8], needle: u8, from: usize) -> Option<usize> {
    bytes[from..].iter().position(|&b| b == needle).map(|p| p + from)
}

/// The index just past the first occurrence of `needle` at or after `from`.
fn find_sub(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POISON_HEAD: &str = r##"
<head>
  <title>Poison</title>
  <meta description="A test effect." />
  <meta publisher="SignalRGB" />
  <meta
    property="bgColor"
    label="Background Color"
    type="color"
    min="0"
    max="360"
    default="#130032"
  />
  <meta property="speedRaw" label="Speed" type="number" min="0" max="100" default="10" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="exCanvas" width="320" height="200"></canvas>
</body>
<script>
  var c = document.getElementById("exCanvas");
  ctx.fillRect(0, 0, width, height);
</script>
"##;

    #[test]
    fn parses_poison_shape() {
        let p = parse(POISON_HEAD);
        assert_eq!(p.title.as_deref(), Some("Poison"));
        assert_eq!(p.description.as_deref(), Some("A test effect."));
        assert_eq!(p.metas.len(), 2);
        assert_eq!(p.metas[0].attr("property"), Some("bgColor"));
        assert_eq!(p.metas[0].attr("type"), Some("color"));
        assert_eq!(p.metas[0].attr("default"), Some("#130032"));
        assert_eq!(p.metas[1].attr("property"), Some("speedRaw"));
        assert_eq!(p.metas[1].attr("default"), Some("10"));
        let canvas = p.canvas.expect("canvas element");
        assert_eq!(canvas.attr("id"), Some("exCanvas"));
        assert_eq!(canvas.attr("width"), Some("320"));
        assert_eq!(canvas.attr("height"), Some("200"));
        assert_eq!(p.scripts.len(), 1);
        assert!(p.scripts[0].contains("getElementById"));
    }

    #[test]
    fn attributes_are_case_insensitive() {
        let p = parse(r#"<meta Property="x" TYPE="number" DEFAULT="3"/>"#);
        assert_eq!(p.metas.len(), 1);
        assert_eq!(p.metas[0].attr("property"), Some("x"));
        assert_eq!(p.metas[0].attr("type"), Some("number"));
        assert_eq!(p.metas[0].attr("default"), Some("3"));
    }

    #[test]
    fn multiple_scripts_keep_order() {
        let p = parse("<script>var a=1;</script><script>var b=2;</script>");
        assert_eq!(p.scripts, vec!["var a=1;".to_string(), "var b=2;".to_string()]);
    }

    #[test]
    fn comments_are_skipped() {
        let p = parse("<!-- <title>No</title> --><title>Yes</title>");
        assert_eq!(p.title.as_deref(), Some("Yes"));
    }

    #[test]
    fn entities_are_unescaped() {
        let p = parse(r#"<meta property="a" label="A &amp; B" type="textfield"/>"#);
        assert_eq!(p.metas[0].attr("label"), Some("A & B"));
    }

    #[test]
    fn quoted_gt_inside_attribute_does_not_end_the_tag() {
        let p = parse(r#"<meta property="a" label="x > y" type="color"/>"#);
        assert_eq!(p.metas.len(), 1);
        assert_eq!(p.metas[0].attr("label"), Some("x > y"));
    }

    #[test]
    fn script_with_src_has_no_body() {
        let p = parse(r#"<script src="x.js"></script>"#);
        assert!(p.scripts.is_empty());
    }
}
