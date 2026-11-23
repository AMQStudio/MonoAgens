//! Terminal graphics detection for the optional inline image preview (kitty graphics protocol
//! or sixel). Detection is environment-based (no terminal query, so it cannot stall or write
//! anything); `MONOAGENS_GRAPHICS=none|kitty|sixel` overrides it.
//!
//! Status: detection and the escape builders exist and are tested; the live feed still draws
//! the existing `WxH` attachment card, because placing and clearing images across ratatui's
//! diff-based redraw is not yet stable enough to ship (task 6.2 is optional).

/// What the terminal can draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Graphics {
    None,
    Kitty,
    Sixel,
}

/// Detects graphics support from environment values.
#[must_use]
pub fn detect(get: impl Fn(&str) -> Option<String>) -> Graphics {
    if let Some(forced) = get("MONOAGENS_GRAPHICS") {
        return match forced.trim().to_ascii_lowercase().as_str() {
            "kitty" => Graphics::Kitty,
            "sixel" => Graphics::Sixel,
            _ => Graphics::None,
        };
    }
    let term = get("TERM").unwrap_or_default().to_ascii_lowercase();
    let program = get("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    if get("TMUX").is_some_and(|v| !v.is_empty()) {
        // Passing graphics through a multiplexer needs extra setup; keep the card.
        return Graphics::None;
    }
    if get("KITTY_WINDOW_ID").is_some() || term.contains("kitty") || matches!(program.as_str(), "wezterm" | "ghostty") {
        return Graphics::Kitty;
    }
    if term.contains("sixel") || matches!(term.as_str(), "foot" | "mlterm" | "yaft-256color") {
        return Graphics::Sixel;
    }
    Graphics::None
}

/// Kitty graphics transmit-and-display sequence for PNG bytes, scaled into `cols` x `rows` cells.
/// Chunked into 4 KiB payloads as the protocol requires.
#[must_use]
pub fn kitty_png_sequence(png: &[u8], cols: u16, rows: u16) -> String {
    let b64 = crate::clipboard::encode_base64(png);
    let chunks: Vec<&str> = b64
        .as_bytes()
        .chunks(4096)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    let mut out = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            out.push_str(&format!("\x1b_Ga=T,f=100,c={cols},r={rows},m={more};{chunk}\x1b\\"));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
    }
    out
}

/// Escape sequences to draw `png` at the cursor for `support`; empty when there is no graphics
/// support, so nothing graphics-related is ever written to a terminal that cannot show it.
#[must_use]
pub fn preview_sequence(support: Graphics, png: &[u8], cols: u16, rows: u16) -> String {
    match support {
        Graphics::Kitty => kitty_png_sequence(png, cols, rows),
        Graphics::Sixel | Graphics::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn detects_kitty_sixel_and_nothing() {
        assert_eq!(detect(env(&[("TERM", "xterm-kitty")])), Graphics::Kitty);
        assert_eq!(detect(env(&[("KITTY_WINDOW_ID", "3"), ("TERM", "xterm-256color")])), Graphics::Kitty);
        assert_eq!(detect(env(&[("TERM", "foot")])), Graphics::Sixel);
        assert_eq!(detect(env(&[("TERM", "xterm-256color")])), Graphics::None);
        assert_eq!(detect(env(&[("TERM", "xterm-kitty"), ("TMUX", "/tmp/x")])), Graphics::None);
        assert_eq!(detect(env(&[("TERM", "xterm-kitty"), ("MONOAGENS_GRAPHICS", "none")])), Graphics::None);
        assert_eq!(detect(env(&[("MONOAGENS_GRAPHICS", "sixel")])), Graphics::Sixel);
    }

    #[test]
    fn no_graphics_support_writes_no_graphics_escape() {
        assert!(preview_sequence(Graphics::None, b"png", 10, 5).is_empty());
        assert!(preview_sequence(Graphics::Sixel, b"png", 10, 5).is_empty(), "sixel encoder not shipped");
        let k = preview_sequence(Graphics::Kitty, &[0u8; 6000], 10, 5);
        assert!(k.starts_with("\x1b_Ga=T,f=100,c=10,r=5,m=1;") && k.ends_with("\x1b\\"));
        assert!(k.contains("\x1b_Gm=0;"), "last chunk closes the transfer");
    }
}
