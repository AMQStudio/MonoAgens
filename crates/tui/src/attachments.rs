//! Pure-Rust image dimension parsing, attachment classification, smart paste handling,
//! workspace-gated host file import, size formatting, and zero-RAM-spike Waist-2 `Percept`
//! attachment staging within the 240 KiB inline cap.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use monoagens_gateway::DEFAULT_CUSTODY_TTL_MS;
use monoagens_protocol::{AttachmentHandle, ModalitySlot, Time};

/// Safe inline payload ceiling (`240 KiB`) keeping well below Gateway's `256 KiB` limit.
pub const MAX_TUI_PERCEPT_INLINE_BYTES: usize = 240 * 1024;

/// Maximum text/code document size (`64 KiB`) inlined directly as `<attached_file>`.
pub const MAX_INLINE_DOCUMENT_BYTES: usize = 64 * 1024;

/// Maximum header bytes read from disk-backed images to parse `WxH` dimensions (`64 KiB`).
const IMAGE_HEADER_READ_BYTES: u64 = 65_536;

/// Supported native image extensions (lowercase).
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// Supported native document extensions (lowercase) that attach without `workspace` plugin.
const NATIVE_DOCUMENT_EXTS: &[&str] = &["pdf"];

/// Kind of pending attachment held in the composer draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAttachmentKind {
    Image {
        width: Option<u32>,
        height: Option<u32>,
        mime: String,
    },
    Document {
        filename: String,
        mime: String,
    },
    LongText {
        line_count: usize,
    },
}

/// Pending attachment card staged in the composer draft.
///
/// Disk-backed files (`source_path = Some(...)`) keep `bytes = None` and record `byte_len`
/// from filesystem metadata to avoid loading large media/PDFs into RAM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttachment {
    pub id: usize,
    pub kind: PendingAttachmentKind,
    pub label: String,
    pub source_path: Option<PathBuf>,
    pub bytes: Option<Vec<u8>>,
    pub byte_len: usize,
}

impl PendingAttachment {
    /// Inline composer token (`📷 #1`, `📄 #2`, `📝 #3`).
    #[must_use]
    pub fn inline_token(&self) -> String {
        format!("{} #{}", self.icon(), self.id)
    }

    /// Emoji/glyph icon for this attachment kind.
    #[must_use]
    pub const fn icon(&self) -> &'static str {
        match &self.kind {
            PendingAttachmentKind::Image { .. } => "📷",
            PendingAttachmentKind::Document { .. } => "📄",
            PendingAttachmentKind::LongText { .. } => "📝",
        }
    }

    /// Middle-row body text shown on the preview card.
    #[must_use]
    pub fn card_body_label(&self) -> String {
        match &self.kind {
            PendingAttachmentKind::Image { .. } => "📷".to_owned(),
            PendingAttachmentKind::Document { filename, .. } => filename.clone(),
            PendingAttachmentKind::LongText { line_count } => format!("{line_count} lines"),
        }
    }

    /// Bottom-border metadata shown on the preview card (`640x640`, `42 KB`, etc.).
    #[must_use]
    pub fn card_footer_label(&self) -> String {
        match &self.kind {
            PendingAttachmentKind::Image {
                width: Some(w),
                height: Some(h),
                ..
            } => format!("{w}x{h}"),
            _ => format_byte_size(self.byte_len),
        }
    }
}

/// Result of classifying a bracketed paste or clipboard text payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteItem {
    Attach(PendingAttachment),
    PlainPath(String),
    PlainText(String),
}

/// Result of inspecting a candidate file path for native media attachment support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAttachmentOutcome {
    Supported(PendingAttachment),
    UnsupportedPath(String),
    NotFound(String),
}

/// Formats a byte count into human-readable `"512 B"`, `"42 KB"`, `"20.0 MB"`.
#[must_use]
pub fn format_byte_size(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        let kb = (bytes as f64 / 1024.0).round() as u64;
        return format!("{} KB", kb.max(1));
    }
    let mb = bytes as f64 / (1024.0 * 1024.0);
    format!("{mb:.1} MB")
}

/// Pure-Rust image dimension parser supporting PNG, GIF, JPEG (SOF0/SOF1/SOF2),
/// and WebP (VP8 / VP8L / VP8X).
#[must_use]
pub fn parse_image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if let Some(dims) = parse_png_dimensions(bytes) {
        return Some(dims);
    }
    if let Some(dims) = parse_gif_dimensions(bytes) {
        return Some(dims);
    }
    if let Some(dims) = parse_jpeg_dimensions(bytes) {
        return Some(dims);
    }
    parse_webp_dimensions(bytes)
}

fn parse_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 {
        return None;
    }
    if &bytes[0..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (w > 0 && h > 0).then_some((w, h))
}

fn parse_gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 {
        return None;
    }
    if &bytes[0..6] != b"GIF87a" && &bytes[0..6] != b"GIF89a" {
        return None;
    }
    let w = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let h = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    (w > 0 && h > 0).then_some((w, h))
}

fn parse_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] != 0xFF {
            i += 1;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;
        if matches!(marker, 0x00 | 0x01 | 0xD0..=0xD8) {
            continue;
        }
        if matches!(marker, 0xD9 | 0xDA) {
            break;
        }
        if i + 2 > bytes.len() {
            break;
        }
        let seg_len = usize::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        if seg_len < 2 || i + seg_len > bytes.len() {
            break;
        }
        if matches!(marker, 0xC0..=0xC2) && seg_len >= 7 && i + 7 <= bytes.len() {
            let h = u32::from(u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]));
            let w = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            if w > 0 && h > 0 {
                return Some((w, h));
            }
        }
        i += seg_len;
    }
    None
}

fn parse_webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8 " => {
            if bytes.len() < 30 || &bytes[23..26] != b"\x9d\x01\x2a" {
                return None;
            }
            let w = u32::from(u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF);
            let h = u32::from(u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF);
            (w > 0 && h > 0).then_some((w, h))
        }
        b"VP8L" => {
            if bytes.len() < 25 || bytes[20] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            Some((w, h))
        }
        b"VP8X" => {
            if bytes.len() < 30 {
                return None;
            }
            let w = (u32::from(bytes[24])
                | (u32::from(bytes[25]) << 8)
                | (u32::from(bytes[26]) << 16))
                + 1;
            let h = (u32::from(bytes[27])
                | (u32::from(bytes[28]) << 8)
                | (u32::from(bytes[29]) << 16))
                + 1;
            Some((w, h))
        }
        _ => None,
    }
}

/// Classifies native media/PDF file extensions (`png`, `jpg`, `jpeg`, `webp`, `gif`, `pdf`)
/// that always attach as numbered preview cards regardless of `workspace` plugin state.
#[must_use]
pub fn classify_native_media_extension(path: &Path) -> Option<(bool, &'static str)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)?;
    if IMAGE_EXTS.contains(&ext.as_str()) {
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "application/octet-stream",
        };
        return Some((true, mime));
    }
    if NATIVE_DOCUMENT_EXTS.contains(&ext.as_str()) {
        return Some((false, "application/pdf"));
    }
    None
}

/// Attempts to build a disk-backed native media/PDF [`PendingAttachment`] without loading
/// the full file into memory (reads at most 64 KiB header for image dimensions).
#[must_use]
pub fn inspect_file_for_attachment(raw_input: &str) -> FileAttachmentOutcome {
    let normalized = normalize_candidate_path(raw_input);
    if normalized.is_empty() {
        return FileAttachmentOutcome::NotFound(raw_input.to_owned());
    }
    let path = PathBuf::from(&normalized);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return FileAttachmentOutcome::NotFound(normalized);
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return FileAttachmentOutcome::UnsupportedPath(normalized);
    }
    let Some((is_image, mime)) = classify_native_media_extension(&path) else {
        return FileAttachmentOutcome::UnsupportedPath(normalized);
    };
    let byte_len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    let filename = sanitize_filename(
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment"),
    );
    let kind = if is_image {
        let dims = read_image_header_dimensions(&path);
        PendingAttachmentKind::Image {
            width: dims.map(|d| d.0),
            height: dims.map(|d| d.1),
            mime: mime.to_owned(),
        }
    } else {
        PendingAttachmentKind::Document {
            filename: filename.clone(),
            mime: mime.to_owned(),
        }
    };
    FileAttachmentOutcome::Supported(PendingAttachment {
        id: 1,
        kind,
        label: filename,
        source_path: Some(path),
        bytes: None,
        byte_len,
    })
}

fn read_image_header_dimensions(path: &Path) -> Option<(u32, u32)> {
    let file = File::open(path).ok()?;
    let mut header = Vec::with_capacity(8192);
    file.take(IMAGE_HEADER_READ_BYTES)
        .read_to_end(&mut header)
        .ok()?;
    parse_image_dimensions(&header)
}

/// Classifies bracketed paste or clipboard text into native attachment cards,
/// non-media local paths (which the caller gates through `workspace`), or plain text.
#[must_use]
pub fn classify_paste_input(raw: &str) -> Vec<PasteItem> {
    let normalized_newlines = raw.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized_newlines.trim();

    if let Some(paths) = try_extract_existing_paths(trimmed) {
        let mut out = Vec::with_capacity(paths.len());
        let mut next_id = 1usize;
        for path in paths {
            let path_str = path.display().to_string();
            match inspect_file_for_attachment(&path_str) {
                FileAttachmentOutcome::Supported(mut att) => {
                    att.id = next_id;
                    next_id += 1;
                    out.push(PasteItem::Attach(att));
                }
                FileAttachmentOutcome::UnsupportedPath(p)
                | FileAttachmentOutcome::NotFound(p) => {
                    out.push(PasteItem::PlainPath(p));
                }
            }
        }
        return out;
    }

    let line_count = normalized_newlines.lines().count();
    let char_count = normalized_newlines.chars().count();
    if line_count >= 8 || char_count >= 600 {
        let bytes = normalized_newlines.into_bytes();
        let byte_len = bytes.len();
        return vec![PasteItem::Attach(PendingAttachment {
            id: 1,
            kind: PendingAttachmentKind::LongText { line_count },
            label: "Pasted text".to_owned(),
            source_path: None,
            bytes: Some(bytes),
            byte_len,
        })];
    }

    vec![PasteItem::PlainText(normalized_newlines)]
}

/// Sanitizes an actor identifier strictly to `[A-Za-z0-9_-]`, matching `plugins/workspace`.
#[must_use]
pub fn sanitize_actor_key(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.trim_matches('_').is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

/// Sanitizes a single filename component (never empty, `.`, `..`, or containing path separators).
#[must_use]
pub fn sanitize_filename(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "file".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Resolves the soul's workspace sandbox root using the exact same convention as `plugins/workspace`:
/// `<config_dir>/plugins/workspace.toml` (`root = "..."`), or `<data_dir>/plugins/workspace/workspaces`.
#[must_use]
pub fn resolve_workspace_root(config_path: &Path, data_dir: &Path) -> PathBuf {
    let cfg_dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let ws_toml = cfg_dir.join("plugins").join("workspace.toml");
    if let Ok(raw) = std::fs::read_to_string(&ws_toml)
        && let Ok(doc) = raw.parse::<toml::Table>()
        && let Some(root) = doc.get("root").and_then(|v| v.as_str())
    {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    data_dir.join("plugins").join("workspace").join("workspaces")
}

/// Imports a host file safely into `<actor_workspace>/inbox/<sanitized_filename>` via streaming
/// copy (refusing symlinks/directories, deduplicating conflicting names with `-2`, `-3`, ...),
/// returning the relative workspace path `inbox/<filename>`.
pub fn import_host_file_to_workspace(
    src_path: &Path,
    config_path: &Path,
    data_dir: &Path,
    actor_id: &str,
) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(src_path)
        .map_err(|e| format!("cannot access '{}': {e}", src_path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "refusing to import symlink '{}'",
            src_path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
        ));
    }
    if !meta.file_type().is_file() {
        return Err(format!(
            "refusing to import directory or special file '{}'",
            src_path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
        ));
    }

    let ws_root = resolve_workspace_root(config_path, data_dir);
    let actor_key = sanitize_actor_key(actor_id);
    let inbox_dir = ws_root.join(actor_key).join("inbox");
    std::fs::create_dir_all(&inbox_dir)
        .map_err(|e| format!("failed to create workspace inbox: {e}"))?;
    if std::fs::symlink_metadata(&inbox_dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("refusing to write into symlinked workspace inbox".to_owned());
    }

    let raw_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let sanitized = sanitize_filename(raw_name);
    let dest_name = copy_deduplicated_file(src_path, meta.len(), &inbox_dir, &sanitized)?;
    Ok(format!("inbox/{dest_name}"))
}

fn copy_deduplicated_file(
    src_path: &Path,
    src_len: u64,
    dir: &Path,
    sanitized_name: &str,
) -> Result<String, String> {
    let (stem, ext) = split_stem_and_ext(sanitized_name);
    for attempt in 1usize..=9_999 {
        let candidate = if attempt == 1 {
            sanitized_name.to_owned()
        } else if let Some(ext) = ext {
            format!("{stem}-{attempt}.{ext}")
        } else {
            format!("{stem}-{attempt}")
        };
        let dest = dir.join(&candidate);
        match std::fs::symlink_metadata(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::copy(src_path, &dest)
                    .map_err(|w| format!("failed to copy to '{}': {w}", dest.display()))?;
                return Ok(candidate);
            }
            Err(e) => {
                return Err(format!("failed to inspect '{}': {e}", dest.display()));
            }
            Ok(existing_meta) => {
                if existing_meta.file_type().is_symlink() || !existing_meta.file_type().is_file() {
                    continue;
                }
                if existing_meta.len() == src_len && files_equal_streaming(src_path, &dest) {
                    return Ok(candidate);
                }
            }
        }
    }
    Err("too many filename collisions in workspace inbox".to_owned())
}

fn write_deduplicated_bytes(dir: &Path, sanitized_name: &str, bytes: &[u8]) -> Result<String, String> {
    let (stem, ext) = split_stem_and_ext(sanitized_name);
    for attempt in 1usize..=9_999 {
        let candidate = if attempt == 1 {
            sanitized_name.to_owned()
        } else if let Some(ext) = ext {
            format!("{stem}-{attempt}.{ext}")
        } else {
            format!("{stem}-{attempt}")
        };
        let dest = dir.join(&candidate);
        match std::fs::symlink_metadata(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&dest, bytes)
                    .map_err(|w| format!("failed to write '{}': {w}", dest.display()))?;
                return Ok(candidate);
            }
            Err(e) => {
                return Err(format!("failed to inspect '{}': {e}", dest.display()));
            }
            Ok(existing_meta) => {
                if existing_meta.file_type().is_symlink() || !existing_meta.file_type().is_file() {
                    continue;
                }
                if existing_meta.len() == bytes.len() as u64
                    && std::fs::read(&dest).ok().as_deref() == Some(bytes)
                {
                    return Ok(candidate);
                }
            }
        }
    }
    Err("too many filename collisions in destination directory".to_owned())
}

fn files_equal_streaming(a: &Path, b: &Path) -> bool {
    let (Ok(fa), Ok(fb)) = (File::open(a), File::open(b)) else {
        return false;
    };
    let mut ra = BufReader::new(fa);
    let mut rb = BufReader::new(fb);
    let mut ba = [0u8; 65_536];
    let mut bb = [0u8; 65_536];
    loop {
        let Ok(na) = read_chunk_full(&mut ra, &mut ba) else {
            return false;
        };
        let Ok(nb) = read_chunk_full(&mut rb, &mut bb) else {
            return false;
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

fn read_chunk_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0usize;
    while total < buf.len() {
        let n = reader.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

fn split_stem_and_ext(name: &str) -> (&str, Option<&str>) {
    if let Some(dot) = name.rfind('.')
        && dot > 0
        && dot + 1 < name.len()
    {
        (&name[..dot], Some(&name[dot + 1..]))
    } else {
        (name, None)
    }
}

fn try_extract_existing_paths(trimmed: &str) -> Option<Vec<PathBuf>> {
    if trimmed.is_empty() {
        return None;
    }

    let whole_norm = normalize_candidate_path(trimmed);
    if looks_like_filesystem_path(trimmed, &whole_norm) {
        let p = PathBuf::from(&whole_norm);
        if p.exists() {
            return Some(vec![p]);
        }
    }

    let mut paths = Vec::new();
    for line in trimmed.lines() {
        let line_trim = line.trim();
        if line_trim.is_empty() {
            continue;
        }
        let line_norm = normalize_candidate_path(line_trim);
        if looks_like_filesystem_path(line_trim, &line_norm) {
            let p = PathBuf::from(&line_norm);
            if p.exists() {
                paths.push(p);
                continue;
            }
        }
        let tokens = split_shell_tokens(line_trim);
        if tokens.is_empty() {
            return None;
        }
        for tok in tokens {
            let norm = normalize_candidate_path(&tok);
            if !looks_like_filesystem_path(&tok, &norm) {
                return None;
            }
            let p = PathBuf::from(&norm);
            if !p.exists() {
                return None;
            }
            paths.push(p);
        }
    }

    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

fn looks_like_filesystem_path(raw: &str, normalized: &str) -> bool {
    raw.starts_with("file://")
        || raw.starts_with("~/")
        || raw.starts_with('/')
        || raw.starts_with("./")
        || raw.starts_with("../")
        || normalized.contains('/')
        || normalized.contains('\\')
}

/// Normalizes `file://` URIs, surrounding `'...'` / `"..."` quotes, `~/`, and `\ ` escapes.
#[must_use]
pub fn normalize_candidate_path(raw: &str) -> String {
    let mut s = raw.trim().to_owned();
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\''))
            || (s.starts_with('"') && s.ends_with('"')))
    {
        s = s[1..s.len() - 1].to_owned();
    } else {
        s = unescape_shell_spaces(&s);
    }

    if let Some(rest) = s
        .strip_prefix("file://localhost")
        .or_else(|| s.strip_prefix("file://"))
    {
        s = percent_decode_utf8(rest);
    }

    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest).display().to_string();
    }

    s
}

fn unescape_shell_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(&next) = chars.peek()
            && matches!(next, ' ' | '\'' | '"' | '\\' | '(' | ')' | '&' | ';')
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

fn split_shell_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            } else {
                cur.push(ch);
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == '\\' {
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn percent_decode_utf8(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut i = 0usize;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let (Some(hi), Some(lo)) = (hex_val(raw[i + 1]), hex_val(raw[i + 2]))
        {
            bytes.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Builds the expanded Percept payload text and Waist-2 `AttachmentHandle`s for a submission,
/// staging media/PDF attachments via streaming `fs::copy` (zero RAM spike) into
/// `<actor_workspace>/inbox/` (when `workspace` is active) or `<data_dir>/tui_attachments/`,
/// never exposing raw host paths, and keeping inline text safely below [`MAX_TUI_PERCEPT_INLINE_BYTES`].
#[must_use]
pub fn build_percept_payload_and_attachments(
    base_text: &str,
    attachments: &[PendingAttachment],
    config_path: &Path,
    data_dir: &Path,
    actor_id: &str,
    workspace_active: bool,
    now_ms: Time,
) -> (String, Vec<AttachmentHandle>) {
    let mut out = base_text.to_owned();
    let mut handles = Vec::new();
    let expires_at_ms = now_ms.saturating_add_ms(DEFAULT_CUSTODY_TTL_MS);

    for att in attachments {
        match &att.kind {
            PendingAttachmentKind::LongText { line_count } => {
                let text = att
                    .bytes
                    .as_deref()
                    .map(String::from_utf8_lossy)
                    .unwrap_or_default();
                let header = format!("\n<attached_text id=\"#{}\" lines=\"{line_count}\">\n", att.id);
                let footer = "\n</attached_text>";
                append_bounded_block(
                    &mut out,
                    &header,
                    &text,
                    footer,
                    att,
                    config_path,
                    data_dir,
                    actor_id,
                    workspace_active,
                    now_ms,
                );
            }
            PendingAttachmentKind::Document { filename, mime } => {
                let safe_name = sanitize_filename(filename);
                let staged_ref = stage_attachment_streaming(
                    att,
                    &safe_name,
                    config_path,
                    data_dir,
                    actor_id,
                    workspace_active,
                    now_ms,
                );
                let handle_id = staged_ref.clone().unwrap_or_else(|| {
                    format!("tui-att-{}-#{}-{safe_name}", now_ms.as_millis(), att.id)
                });
                handles.push(AttachmentHandle {
                    handle_id: handle_id.clone(),
                    modality_slot: ModalitySlot::SensorBlob,
                    byte_len: att.byte_len as u64,
                    expires_at_ms,
                    embed_model_ver: String::new(),
                });

                let is_pdf = mime.eq_ignore_ascii_case("application/pdf");
                if is_pdf {
                    let note = format!(
                        "\n[Attachment #{}: PDF \"{}\" ({}) handle={}]",
                        att.id,
                        safe_name,
                        format_byte_size(att.byte_len),
                        handle_id
                    );
                    append_within_cap(&mut out, &note);
                } else if att.byte_len <= MAX_INLINE_DOCUMENT_BYTES {
                    let small_bytes = att.bytes.clone().or_else(|| {
                        att.source_path
                            .as_ref()
                            .and_then(|p| std::fs::read(p).ok())
                    });
                    if let Some(raw_bytes) = small_bytes
                        && let Ok(utf8) = std::str::from_utf8(&raw_bytes)
                    {
                        let header = format!(
                            "\n<attached_file id=\"#{}\" path=\"{}\" name=\"{}\">\n",
                            att.id, handle_id, safe_name
                        );
                        let footer = "\n</attached_file>";
                        append_bounded_block(
                            &mut out,
                            &header,
                            utf8,
                            footer,
                            att,
                            config_path,
                            data_dir,
                            actor_id,
                            workspace_active,
                            now_ms,
                        );
                    } else {
                        let note = format!(
                            "\n[Attachment #{}: document \"{}\" ({}) handle={}]",
                            att.id,
                            safe_name,
                            format_byte_size(att.byte_len),
                            handle_id
                        );
                        append_within_cap(&mut out, &note);
                    }
                } else {
                    let note = format!(
                        "\n[Attachment #{}: document \"{}\" ({}) handle={}]",
                        att.id,
                        safe_name,
                        format_byte_size(att.byte_len),
                        handle_id
                    );
                    append_within_cap(&mut out, &note);
                }
            }
            PendingAttachmentKind::Image { width, height, .. } => {
                let safe_name = sanitize_filename(&att.label);
                let staged_ref = stage_attachment_streaming(
                    att,
                    &safe_name,
                    config_path,
                    data_dir,
                    actor_id,
                    workspace_active,
                    now_ms,
                );
                let handle_id = staged_ref.clone().unwrap_or_else(|| {
                    format!("tui-img-{}-#{}-{safe_name}", now_ms.as_millis(), att.id)
                });
                handles.push(AttachmentHandle {
                    handle_id: handle_id.clone(),
                    modality_slot: ModalitySlot::Image,
                    byte_len: att.byte_len as u64,
                    expires_at_ms,
                    embed_model_ver: String::new(),
                });
                let detail = match (width, height) {
                    (Some(w), Some(h)) => format!("{w}x{h}"),
                    _ => format_byte_size(att.byte_len),
                };
                let note = format!(
                    "\n[Attachment #{}: image \"{}\" ({}) handle={}]",
                    att.id, safe_name, detail, handle_id
                );
                append_within_cap(&mut out, &note);
            }
        }
    }

    if out.len() > MAX_TUI_PERCEPT_INLINE_BYTES {
        let mut cut = MAX_TUI_PERCEPT_INLINE_BYTES;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }

    (out, handles)
}

/// Stages `att` into workspace `inbox/` (if active) or `<data_dir>/tui_attachments/`,
/// streaming via `std::fs::copy` for disk-backed files so large PDFs/images never spike RAM.
fn stage_attachment_streaming(
    att: &PendingAttachment,
    safe_name: &str,
    config_path: &Path,
    data_dir: &Path,
    actor_id: &str,
    workspace_active: bool,
    now_ms: Time,
) -> Option<String> {
    if workspace_active {
        let ws_root = resolve_workspace_root(config_path, data_dir);
        let inbox_dir = ws_root.join(sanitize_actor_key(actor_id)).join("inbox");
        if std::fs::create_dir_all(&inbox_dir).is_ok() {
            if let Some(bytes) = &att.bytes
                && let Ok(name) = write_deduplicated_bytes(&inbox_dir, safe_name, bytes)
            {
                return Some(format!("inbox/{name}"));
            }
            if let Some(src) = &att.source_path
                && let Ok(meta) = std::fs::symlink_metadata(src)
                && !meta.file_type().is_symlink()
                && meta.file_type().is_file()
                && let Ok(name) = copy_deduplicated_file(src, meta.len(), &inbox_dir, safe_name)
            {
                return Some(format!("inbox/{name}"));
            }
        }
    }

    let tui_dir = data_dir.join("tui_attachments");
    if std::fs::create_dir_all(&tui_dir).is_err() {
        return None;
    }
    let staged_name = format!("{}_{}_{}", now_ms.as_millis(), att.id, safe_name);
    let dest = tui_dir.join(&staged_name);
    if let Some(bytes) = &att.bytes {
        if std::fs::write(&dest, bytes).is_ok() {
            return Some(format!("tui_attachments/{staged_name}"));
        }
    } else if let Some(src) = &att.source_path
        && std::fs::copy(src, &dest).is_ok()
    {
        return Some(format!("tui_attachments/{staged_name}"));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn append_bounded_block(
    out: &mut String,
    header: &str,
    body: &str,
    footer: &str,
    att: &PendingAttachment,
    config_path: &Path,
    data_dir: &Path,
    actor_id: &str,
    workspace_active: bool,
    now_ms: Time,
) {
    let needed = header.len() + body.len() + footer.len();
    if out.len() + needed <= MAX_TUI_PERCEPT_INLINE_BYTES {
        out.push_str(header);
        out.push_str(body);
        out.push_str(footer);
        return;
    }

    let safe_name = sanitize_filename(&format!("{}.txt", att.label));
    let staged_ref = stage_attachment_streaming(
        att,
        &safe_name,
        config_path,
        data_dir,
        actor_id,
        workspace_active,
        now_ms,
    );
    let notice = match &staged_ref {
        Some(r) => format!("\n...[truncated to fit 240 KiB cap; full content staged at {r}]"),
        None => "\n...[truncated to fit 240 KiB cap]".to_owned(),
    };
    let overhead = header.len() + notice.len() + footer.len();
    let avail = MAX_TUI_PERCEPT_INLINE_BYTES.saturating_sub(out.len() + overhead);
    if avail == 0 {
        if let Some(r) = staged_ref {
            let ref_note = format!(
                "\n[Attachment #{}: {} ({})]",
                att.id,
                r,
                format_byte_size(att.byte_len)
            );
            append_within_cap(out, &ref_note);
        }
        return;
    }
    let mut cut = avail.min(body.len());
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    out.push_str(header);
    out.push_str(&body[..cut]);
    out.push_str(&notice);
    out.push_str(footer);
}

fn append_within_cap(out: &mut String, snippet: &str) {
    if out.len() + snippet.len() <= MAX_TUI_PERCEPT_INLINE_BYTES {
        out.push_str(snippet);
    }
}
