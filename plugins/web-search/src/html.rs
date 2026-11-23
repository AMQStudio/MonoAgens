//! Clean HTML-to-text extractor and entity decoder for `web.fetch` and search fallback.
//!
//! Strips `<script>`, `<style>`, `<noscript>`, `<template>`, `<svg>`, comments, and tags,
//! extracts `<title>`, decodes common named and numeric HTML entities, collapses whitespace,
//! and truncates cleanly to `max_chars`.

/// Extracted plain-text representation of an HTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    pub title: String,
    pub text: String,
    pub truncated: bool,
}

const DROP_TAGS: &[&str] = &["script", "style", "noscript", "template", "svg"];
const BLOCK_TAGS: &[&str] = &[
    "address", "article", "aside", "blockquote", "br", "dd", "div", "dl", "dt", "fieldset",
    "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header",
    "hr", "li", "main", "nav", "ol", "p", "pre", "section", "table", "tbody", "td", "tfoot",
    "th", "thead", "tr", "ul",
];

/// Extracts `<title>` and readable body text from `html`, collapsing whitespace and
/// truncating cleanly to at most `max_chars` Unicode scalar values.
#[must_use]
pub fn extract_html_text(html: &str, max_chars: usize) -> ExtractedPage {
    let mut raw_text = String::with_capacity(html.len().min(32_768));
    let mut raw_title = String::new();

    let mut drop_depth: usize = 0;
    let mut in_title = false;
    let mut pos = 0usize;
    let bytes = html.as_bytes();

    while pos < bytes.len() {
        if let Some(rel_lt) = html[pos..].find('<') {
            let chunk = &html[pos..pos + rel_lt];
            if drop_depth == 0 && !chunk.is_empty() {
                if in_title {
                    raw_title.push_str(chunk);
                } else {
                    raw_text.push_str(chunk);
                }
            }
            pos += rel_lt;

            if html[pos..].starts_with("<!--") {
                if let Some(end_comment) = html[pos + 4..].find("-->") {
                    pos = pos + 4 + end_comment + 3;
                } else {
                    break;
                }
                continue;
            }

            let Some(tag_end_rel) = find_tag_end(&html[pos..]) else {
                // Unclosed `<`; treat remainder as plain text if not dropping.
                if drop_depth == 0 {
                    if in_title {
                        raw_title.push_str(&html[pos..]);
                    } else {
                        raw_text.push_str(&html[pos..]);
                    }
                }
                break;
            };

            let tag_inner = html[pos + 1..pos + tag_end_rel].trim();
            pos += tag_end_rel + 1;

            if tag_inner.is_empty() || tag_inner.starts_with('!') || tag_inner.starts_with('?') {
                continue;
            }

            let is_closing = tag_inner.starts_with('/');
            let is_self_closing = !is_closing && tag_inner.ends_with('/');
            let name_slice = tag_inner.trim_start_matches('/').trim_end_matches('/');
            let tag_name = name_slice
                .split(|c: char| c.is_ascii_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if tag_name.is_empty() {
                continue;
            }

            if DROP_TAGS.contains(&tag_name.as_str()) {
                if is_closing {
                    drop_depth = drop_depth.saturating_sub(1);
                } else if !is_self_closing {
                    drop_depth = drop_depth.saturating_add(1);
                }
                continue;
            }

            if tag_name == "title" && drop_depth == 0 {
                if is_closing {
                    in_title = false;
                } else if !is_self_closing {
                    in_title = true;
                }
                continue;
            }

            if drop_depth == 0 && !in_title && BLOCK_TAGS.contains(&tag_name.as_str()) {
                raw_text.push(' ');
            }
        } else {
            let chunk = &html[pos..];
            if drop_depth == 0 {
                if in_title {
                    raw_title.push_str(chunk);
                } else {
                    raw_text.push_str(chunk);
                }
            }
            break;
        }
    }

    let title = collapse_whitespace(&decode_html_entities(&raw_title));
    let decoded_body = decode_html_entities(&raw_text);
    let (text, truncated) = normalize_plain_text(&decoded_body, max_chars);

    ExtractedPage {
        title,
        text,
        truncated,
    }
}

/// Collapses runs of whitespace into single spaces, trims, and truncates to `max_chars`.
#[must_use]
pub fn normalize_plain_text(raw: &str, max_chars: usize) -> (String, bool) {
    let collapsed = collapse_whitespace(raw);
    truncate_chars(&collapsed, max_chars)
}

/// Collapses any whitespace run into a single ASCII space and trims leading/trailing whitespace.
#[must_use]
pub fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = true;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Truncates `text` cleanly to at most `max_chars` characters, returning `(slice, truncated)`.
#[must_use]
pub fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    if max_chars == 0 {
        return (String::new(), !text.is_empty());
    }
    for (count, (idx, _)) in text.char_indices().enumerate() {
        if count == max_chars {
            return (text[..idx].trim_end().to_owned(), true);
        }
    }
    (text.to_owned(), false)
}

/// Decodes common named (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&nbsp;`, etc.)
/// and numeric (`&#123;`, `&#x7B;`) HTML entities.
#[must_use]
pub fn decode_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }
    let mut out = String::with_capacity(input.len());
    let mut pos = 0usize;

    while let Some(rel_amp) = input[pos..].find('&') {
        out.push_str(&input[pos..pos + rel_amp]);
        pos += rel_amp;

        let rest = &input[pos..];
        if let Some(semi) = rest[1..].find(';').filter(|idx| *idx <= 16) {
            let entity = &rest[1..1 + semi];
            if let Some(decoded) = decode_entity_body(entity) {
                out.push(decoded);
                pos += 2 + semi;
                continue;
            }
        }

        out.push('&');
        pos += 1;
    }
    out.push_str(&input[pos..]);
    out
}

fn decode_entity_body(entity: &str) -> Option<char> {
    if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16).ok()?;
        return char::from_u32(code).filter(|c| !c.is_control() || *c == '\n' || *c == '\t');
    }
    if let Some(dec) = entity.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        return char::from_u32(code).filter(|c| !c.is_control() || *c == '\n' || *c == '\t');
    }
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "hellip" => Some('\u{2026}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201C}'),
        "rdquo" => Some('\u{201D}'),
        "copy" => Some('\u{00A9}'),
        "reg" => Some('\u{00AE}'),
        "trade" => Some('\u{2122}'),
        "bull" => Some('\u{2022}'),
        "middot" => Some('\u{00B7}'),
        "laquo" => Some('\u{00AB}'),
        "raquo" => Some('\u{00BB}'),
        _ => None,
    }
}

fn find_tag_end(slice: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (idx, byte) in slice.bytes().enumerate().skip(1) {
        match quote {
            Some(q) if byte == q => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'>' => return Some(idx),
            None => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_strips_scripts_styles_and_decodes_entities() {
        let html = r#"<!doctype html>
        <html>
          <head>
            <title>Rust &amp; Safety &#x2014; Guide</title>
            <style>body { color: red; }</style>
            <script>console.log("secret <tag>");</script>
          </head>
          <body>
            <h1>Hello&nbsp;World</h1>
            <!-- hidden comment -->
            <p>First &lt;paragraph&gt; &quot;quoted&quot; &#39;ok&#39;.</p>
            <noscript>Enable JS</noscript>
            <svg><text>Ignored SVG</text></svg>
            <div>Second   line   here.</div>
          </body>
        </html>"#;

        let extracted = extract_html_text(html, 12_000);
        assert_eq!(extracted.title, "Rust & Safety \u{2014} Guide");
        assert_eq!(
            extracted.text,
            "Hello World First <paragraph> \"quoted\" 'ok'. Second line here."
        );
        assert!(!extracted.truncated);
    }

    #[test]
    fn truncates_cleanly_at_max_chars() {
        let html = "<p>Alpha Beta Gamma Delta</p>";
        let extracted = extract_html_text(html, 10);
        assert_eq!(extracted.text, "Alpha Beta");
        assert!(extracted.truncated);
    }
}
