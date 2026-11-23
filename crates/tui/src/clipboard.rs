//! Cross-platform clipboard helpers: pure-Rust Base64 + OSC 52 terminal sequence
//! (with tmux DCS passthrough) and best-effort system clipboard read/write.

use std::io::{self, Write};
use std::process::{Command, Stdio};

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Content read from the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    ImagePng(Vec<u8>),
    Text(String),
}

/// Pure-Rust standard Base64 encoder (RFC 4648).
#[must_use]
pub fn encode_base64(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(B64_ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Formats an OSC 52 clipboard sequence (`\x1b]52;c;<base64>\x07`), wrapping in a
/// tmux DCS passthrough sequence when `TMUX` is present in the environment.
#[must_use]
pub fn osc52_sequence(text: &str) -> String {
    osc52_sequence_with_tmux(text, std::env::var_os("TMUX").is_some())
}

/// Formats an OSC 52 sequence with explicit tmux passthrough control (deterministic in tests).
#[must_use]
pub fn osc52_sequence_with_tmux(text: &str, in_tmux: bool) -> String {
    let b64 = encode_base64(text.as_bytes());
    if in_tmux {
        format!("\x1bPtmux;\x1b\x1b]52;c;{b64}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{b64}\x07")
    }
}

/// Copies `text` to the terminal/system clipboard via OSC 52 (written & flushed to stdout)
/// and best-effort native helper (`wl-copy`, `xclip`, `xsel`, or `pbcopy`).
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let seq = osc52_sequence(text);
    let mut stdout = io::stdout();
    stdout
        .write_all(seq.as_bytes())
        .map_err(|e| format!("failed to write OSC 52 sequence: {e}"))?;
    stdout
        .flush()
        .map_err(|e| format!("failed to flush OSC 52 sequence: {e}"))?;

    // Best-effort native clipboard sync (never fails if helper binaries are absent).
    let candidates: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
    ];
    for (bin, args) in candidates {
        if try_pipe_stdin(bin, args, text.as_bytes()) {
            break;
        }
    }
    Ok(())
}

/// Best-effort reads PNG image bytes or UTF-8 text from the system clipboard.
#[must_use]
pub fn read_clipboard_image_or_text() -> Option<ClipboardContent> {
    let image_cmds: &[(&str, &[&str])] = &[
        ("wl-paste", &["--no-newline", "--type", "image/png"]),
        ("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]),
    ];
    for (bin, args) in image_cmds {
        if let Some(bytes) = try_read_stdout(bin, args)
            && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            return Some(ClipboardContent::ImagePng(bytes));
        }
    }

    let text_cmds: &[(&str, &[&str])] = &[
        ("wl-paste", &["--no-newline"]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
        ("pbpaste", &[]),
    ];
    for (bin, args) in text_cmds {
        if let Some(bytes) = try_read_stdout(bin, args)
            && !bytes.is_empty()
        {
            return Some(ClipboardContent::Text(
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
    }
    None
}

fn try_pipe_stdin(bin: &str, args: &[&str], data: &[u8]) -> bool {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(data).is_err()
    {
        let _ = child.wait();
        return false;
    }
    child.wait().is_ok_and(|s| s.success())
}

fn try_read_stdout(bin: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() && !output.stdout.is_empty() {
        Some(output.stdout)
    } else {
        None
    }
}
