//! Pure-Rust Markdown renderer for assistant replies in the Ratatui chat viewport.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::ThemePalette;

/// Renders Markdown `text` into styled, word-wrapped [`Line`]s constrained to `max_width` display columns.
#[must_use]
pub fn render_markdown_lines(
    text: &str,
    max_width: usize,
    palette: &ThemePalette,
) -> Vec<Line<'static>> {
    let width = max_width.max(4);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();

    for raw_line in text.split('\n') {
        let trimmed = raw_line.trim();
        if let Some(fence_rest) = parse_fence_line(trimmed) {
            if !in_code_block {
                in_code_block = true;
                code_lang = fence_rest.to_owned();
                code_lines.clear();
            } else {
                flush_code_block(&mut out, &code_lang, &code_lines, width, palette);
                in_code_block = false;
                code_lang.clear();
                code_lines.clear();
            }
            continue;
        }

        if in_code_block {
            code_lines.push(raw_line.to_owned());
            continue;
        }

        if trimmed.is_empty() {
            if out.last().is_some_and(|l| !l.spans.is_empty()) {
                out.push(Line::default());
            }
            continue;
        }

        if is_horizontal_rule(trimmed) {
            let rule_w = width.min(56);
            out.push(Line::from(Span::styled(
                "─".repeat(rule_w),
                Style::default().fg(palette.separator),
            )));
            continue;
        }

        if let Some((level, heading_text)) = parse_heading(raw_line) {
            if out.last().is_some_and(|l| !l.spans.is_empty()) {
                out.push(Line::default());
            }
            let h_color = if level <= 2 {
                palette.accent
            } else {
                palette.fg
            };
            let base = Style::default().fg(h_color).add_modifier(Modifier::BOLD);
            let spans = parse_inline_spans(heading_text, base, palette);
            wrap_styled_spans(&spans, &[], &[], width, &mut out);
            continue;
        }

        if let Some(quote_text) = parse_blockquote(raw_line) {
            let prefix = vec![Span::styled(
                "▎ ".to_owned(),
                Style::default().fg(palette.accent),
            )];
            let base = Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::ITALIC);
            let spans = parse_inline_spans(quote_text, base, palette);
            wrap_styled_spans(&spans, &prefix, &prefix, width, &mut out);
            continue;
        }

        if let Some((bullet_prefix, body)) = parse_list_item(raw_line) {
            let prefix_w = bullet_prefix.width();
            let first_prefix = vec![Span::styled(
                bullet_prefix,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )];
            let cont_prefix = vec![Span::raw(" ".repeat(prefix_w))];
            let base = Style::default().fg(palette.fg);
            let spans = parse_inline_spans(body, base, palette);
            wrap_styled_spans(&spans, &first_prefix, &cont_prefix, width, &mut out);
            continue;
        }

        let base = Style::default().fg(palette.fg);
        let spans = parse_inline_spans(raw_line, base, palette);
        wrap_styled_spans(&spans, &[], &[], width, &mut out);
    }

    if in_code_block {
        flush_code_block(&mut out, &code_lang, &code_lines, width, palette);
    }

    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn parse_fence_line(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with("```") {
        let rest = trimmed.trim_start_matches('`').trim();
        // If there are backticks inside the rest, it's inline code on a single line, not a fence.
        if rest.contains('`') {
            return None;
        }
        Some(rest)
    } else {
        None
    }
}

fn flush_code_block(
    out: &mut Vec<Line<'static>>,
    lang: &str,
    code_lines: &[String],
    max_width: usize,
    palette: &ThemePalette,
) {
    let code_bg = palette.sidebar_bg;
    let border_style = Style::default().fg(palette.separator).bg(code_bg);
    let label_style = Style::default()
        .fg(palette.accent)
        .bg(code_bg)
        .add_modifier(Modifier::BOLD);
    let code_style = Style::default().fg(palette.fg).bg(code_bg);

    let max_content_w = code_lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(lang.width() + 4);
    let box_w = (max_content_w + 3).clamp(16, max_width);
    let inner_code_w = box_w.saturating_sub(3).max(1);

    // Top border: ╭─ lang ────────
    let header_title = if lang.is_empty() {
        "╭─".to_owned()
    } else {
        let max_lang = box_w.saturating_sub(5);
        let trunc = truncate_display(lang, max_lang);
        format!("╭─ {trunc} ")
    };
    let top_dashes = box_w.saturating_sub(header_title.width());
    if lang.is_empty() {
        out.push(Line::from(vec![Span::styled(
            format!("{header_title}{}", "─".repeat(top_dashes)),
            border_style,
        )]));
    } else {
        let max_lang = box_w.saturating_sub(5);
        let trunc = truncate_display(lang, max_lang);
        out.push(Line::from(vec![
            Span::styled("╭─ ".to_owned(), border_style),
            Span::styled(trunc, label_style),
            Span::styled(
                format!(" {}", "─".repeat(top_dashes)),
                border_style,
            ),
        ]));
    }

    if code_lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ".to_owned(), border_style),
            Span::styled(" ".repeat(inner_code_w + 1), code_style),
        ]));
    } else {
        for raw in code_lines {
            for wrapped in wrap_raw_chars(raw, inner_code_w) {
                let pad = inner_code_w.saturating_sub(wrapped.width()) + 1;
                out.push(Line::from(vec![
                    Span::styled("│ ".to_owned(), border_style),
                    Span::styled(format!("{wrapped}{}", " ".repeat(pad)), code_style),
                ]));
            }
        }
    }

    // Bottom border: ╰───────────────
    let bot_dashes = box_w.saturating_sub(1);
    out.push(Line::from(vec![Span::styled(
        format!("╰{}", "─".repeat(bot_dashes)),
        border_style,
    )]));
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 3 {
        return false;
    }
    let first = compact.chars().next().unwrap_or(' ');
    matches!(first, '-' | '*' | '_') && compact.chars().all(|c| c == first)
}

fn parse_heading(raw: &str) -> Option<(usize, &str)> {
    let trimmed = raw.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        Some((hashes, trimmed[hashes + 1..].trim_start()))
    } else {
        None
    }
}

fn parse_blockquote(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_start();
    trimmed
        .strip_prefix("> ")
        .or_else(|| trimmed.strip_prefix('>'))
}

fn parse_list_item(raw: &str) -> Option<(String, &str)> {
    let leading_spaces = raw.bytes().take_while(|&b| b == b' ').count();
    let indent = " ".repeat((leading_spaces / 2) * 2);
    let rest = &raw[leading_spaces..];

    if let Some(body) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        return Some((format!("{indent}• "), body));
    }

    let digit_len = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len > 0 && rest[digit_len..].starts_with(". ") {
        let num = &rest[..digit_len];
        let body = &rest[digit_len + 2..];
        return Some((format!("{indent}{num}. "), body));
    }

    None
}

fn parse_inline_spans(text: &str, base: Style, palette: &ThemePalette) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    let bytes = text.as_bytes();

    let code_style = if palette.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .bg(palette.selection_bg)
            .fg(palette.accent)
    };

    let flush_plain = |buf: &mut String, out: &mut Vec<Span<'static>>| {
        if !buf.is_empty() {
            out.push(Span::styled(std::mem::take(buf), base));
        }
    };

    while i < text.len() {
        // Inline code: `...`
        if bytes[i] == b'`'
            && let Some(rel_end) = text[i + 1..].find('`')
        {
            flush_plain(&mut plain, &mut spans);
            let inner = &text[i + 1..i + 1 + rel_end];
            spans.push(Span::styled(inner.to_owned(), code_style));
            i = i + 1 + rel_end + 1;
            continue;
        }

        // Bold: **...** or __...__
        if (text[i..].starts_with("**") || text[i..].starts_with("__")) && i + 2 < text.len() {
            let delim = &text[i..i + 2];
            if let Some(rel_end) = text[i + 2..].find(delim)
                && rel_end > 0
            {
                flush_plain(&mut plain, &mut spans);
                let inner = &text[i + 2..i + 2 + rel_end];
                let bold_style = base.add_modifier(Modifier::BOLD);
                spans.extend(parse_inline_spans(inner, bold_style, palette));
                i = i + 2 + rel_end + 2;
                continue;
            }
        }

        // Strikethrough: ~~...~~
        if text[i..].starts_with("~~")
            && i + 2 < text.len()
            && let Some(rel_end) = text[i + 2..].find("~~")
            && rel_end > 0
        {
            flush_plain(&mut plain, &mut spans);
            let inner = &text[i + 2..i + 2 + rel_end];
            let strike_style = base.add_modifier(Modifier::CROSSED_OUT);
            spans.extend(parse_inline_spans(inner, strike_style, palette));
            i = i + 2 + rel_end + 2;
            continue;
        }

        // Italic: *...* or _..._ (avoiding snake_case mid-word underscores)
        if bytes[i] == b'*'
            && i + 1 < text.len()
            && bytes[i + 1] != b' '
            && let Some(rel_end) = text[i + 1..].find('*')
            && rel_end > 0
        {
            flush_plain(&mut plain, &mut spans);
            let inner = &text[i + 1..i + 1 + rel_end];
            let italic_style = base.add_modifier(Modifier::ITALIC);
            spans.extend(parse_inline_spans(inner, italic_style, palette));
            i = i + 1 + rel_end + 1;
            continue;
        }

        if bytes[i] == b'_'
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && i + 1 < text.len()
            && bytes[i + 1] != b' '
            && let Some(rel_end) = find_closing_underscore(&text[i + 1..])
            && rel_end > 0
        {
            flush_plain(&mut plain, &mut spans);
            let inner = &text[i + 1..i + 1 + rel_end];
            let italic_style = base.add_modifier(Modifier::ITALIC);
            spans.extend(parse_inline_spans(inner, italic_style, palette));
            i = i + 1 + rel_end + 1;
            continue;
        }

        if let Some(ch) = text[i..].chars().next() {
            plain.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn find_closing_underscore(slice: &str) -> Option<usize> {
    let bytes = slice.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            let next_ok = bytes
                .get(idx + 1)
                .is_none_or(|nb| !nb.is_ascii_alphanumeric());
            if next_ok {
                return Some(idx);
            }
        }
    }
    None
}

fn wrap_styled_spans(
    spans: &[Span<'static>],
    first_prefix: &[Span<'static>],
    cont_prefix: &[Span<'static>],
    max_width: usize,
    out: &mut Vec<Line<'static>>,
) {
    let first_prefix_w: usize = first_prefix.iter().map(|s| s.content.width()).sum();
    let cont_prefix_w: usize = cont_prefix.iter().map(|s| s.content.width()).sum();

    let mut is_first_line = true;
    let mut cur_spans: Vec<Span<'static>> = first_prefix.to_vec();
    let mut cur_w = first_prefix_w;

    let push_line = |spans_buf: &mut Vec<Span<'static>>,
                     first_flag: &mut bool,
                     width_acc: &mut usize,
                     dest: &mut Vec<Line<'static>>| {
        dest.push(Line::from(std::mem::take(spans_buf)));
        *first_flag = false;
        *spans_buf = cont_prefix.to_vec();
        *width_acc = cont_prefix_w;
    };

    for span in spans {
        let style = span.style;
        let text = span.content.as_ref();
        if text.is_empty() {
            continue;
        }
        for token in text.split_inclusive(' ') {
            let token_w = token.width();
            let line_prefix_w = if is_first_line {
                first_prefix_w
            } else {
                cont_prefix_w
            };
            let avail_w = max_width.saturating_sub(line_prefix_w).max(1);

            if cur_w > line_prefix_w && cur_w + token_w > max_width {
                // Trim trailing space on previous plain span if we're wrapping
                if let Some(last) = cur_spans.last_mut()
                    && last.content.ends_with(' ')
                {
                    let trimmed = last.content.trim_end_matches(' ').to_owned();
                    last.content = trimmed.into();
                }
                push_line(&mut cur_spans, &mut is_first_line, &mut cur_w, out);
                let trimmed_start = token.trim_start_matches(' ');
                if trimmed_start.is_empty() {
                    continue;
                }
                append_token_wrapping(
                    trimmed_start,
                    style,
                    avail_w,
                    max_width,
                    &mut is_first_line,
                    &mut cur_spans,
                    &mut cur_w,
                    cont_prefix,
                    cont_prefix_w,
                    out,
                );
            } else {
                append_token_wrapping(
                    token,
                    style,
                    avail_w,
                    max_width,
                    &mut is_first_line,
                    &mut cur_spans,
                    &mut cur_w,
                    cont_prefix,
                    cont_prefix_w,
                    out,
                );
            }
        }
    }

    if cur_spans.len() > if is_first_line { first_prefix.len() } else { cont_prefix.len() }
        || is_first_line
    {
        out.push(Line::from(cur_spans));
    }
}

#[allow(clippy::too_many_arguments)]
fn append_token_wrapping(
    token: &str,
    style: Style,
    avail_w: usize,
    max_width: usize,
    is_first_line: &mut bool,
    cur_spans: &mut Vec<Span<'static>>,
    cur_w: &mut usize,
    cont_prefix: &[Span<'static>],
    cont_prefix_w: usize,
    out: &mut Vec<Line<'static>>,
) {
    if token.width() <= avail_w && *cur_w + token.width() <= max_width {
        push_styled_text(cur_spans, token, style);
        *cur_w += token.width();
        return;
    }

    let mut chunk = String::new();
    for ch in token.chars() {
        let cw = ch.width().unwrap_or(0);
        if *cur_w + chunk.width() + cw > max_width && (!chunk.is_empty() || *cur_w > cont_prefix_w)
        {
            if !chunk.is_empty() {
                push_styled_text(cur_spans, &chunk, style);
                chunk.clear();
            }
            out.push(Line::from(std::mem::take(cur_spans)));
            *is_first_line = false;
            *cur_spans = cont_prefix.to_vec();
            *cur_w = cont_prefix_w;
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() {
        *cur_w += chunk.width();
        push_styled_text(cur_spans, &chunk, style);
    }
}

fn push_styled_text(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        let mut owned = std::mem::take(&mut last.content).into_owned();
        owned.push_str(text);
        last.content = owned.into();
    } else {
        spans.push(Span::styled(text.to_owned(), style));
    }
}

fn wrap_raw_chars(line: &str, max_w: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in line.chars() {
        let cw = ch.width().unwrap_or(0);
        if cur_w + cw > max_w && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += cw;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn truncate_display(s: &str, max_w: usize) -> String {
    if s.width() <= max_w {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if cur_w + cw + 1 > max_w {
            out.push('…');
            break;
        }
        out.push(ch);
        cur_w += cw;
    }
    out
}
