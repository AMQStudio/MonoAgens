//! Composer history search (`Ctrl+R`), recall of a cleared draft and the external editor
//! (`Ctrl+G`). Pure state and helpers; key wiring lives in [`crate::conv`].

use std::path::{Path, PathBuf};
use std::process::Command;

/// Reverse incremental search over the composer history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySearch {
    /// What the user typed.
    pub query: String,
    /// Composer text before the search started (restored on Esc).
    pub saved_text: String,
    /// How many older matches were skipped with `Ctrl+R`.
    pub skip: usize,
}

impl HistorySearch {
    #[must_use]
    pub fn start(current_text: &str) -> Self {
        Self {
            query: String::new(),
            saved_text: current_text.to_owned(),
            skip: 0,
        }
    }

    /// Index into `history` of the match to show: newest first, `skip` older ones passed over.
    /// An empty query matches everything, so `Ctrl+R` alone walks back through the history.
    #[must_use]
    pub fn matched(&self, history: &[String]) -> Option<usize> {
        let needle = self.query.to_lowercase();
        history
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, h)| needle.is_empty() || h.to_lowercase().contains(&needle))
            .nth(self.skip)
            .map(|(i, _)| i)
    }

    /// The text the composer shows while searching.
    #[must_use]
    pub fn preview<'a>(&self, history: &'a [String]) -> Option<&'a str> {
        self.matched(history).map(|i| history[i].as_str())
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.skip = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.skip = 0;
    }

    /// `Ctrl+R` again: the next older match (stays on the oldest one).
    pub fn older(&mut self, history: &[String]) {
        let next = HistorySearch { skip: self.skip + 1, ..self.clone() };
        if next.matched(history).is_some() {
            self.skip += 1;
        }
    }
}

/// Where the external-editor draft lives: `<data_dir>/cache/` (never `/tmp`).
#[must_use]
pub fn editor_draft_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cache").join(format!("draft-{}.md", std::process::id()))
}

/// Editor command from `$VISUAL`, else `$EDITOR`.
#[must_use]
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> Option<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Writes `draft` to `path` (mode 0600), runs `editor` on it through the shell, waits, and
/// returns the edited text. The caller has already left raw mode and the alternate screen.
///
/// # Errors
/// File or spawn failures, or a non-zero editor exit.
pub fn run_external_editor(editor: &str, path: &Path, draft: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    }
    std::fs::write(path, format!("{draft}\n")).map_err(|e| format!("cannot write '{}': {e}", path.display()))?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(path)
        .status()
        .map_err(|e| format!("cannot run '{editor}': {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(path);
        return Err(format!("'{editor}' exited with {status}"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read the edited draft: {e}"))?;
    let _ = std::fs::remove_file(path);
    // Editors add a final newline; the composer does not want one.
    Ok(text.strip_suffix('\n').unwrap_or(&text).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist() -> Vec<String> {
        vec!["alpha".into(), "beta".into(), "alpine skiing".into()]
    }

    #[test]
    fn finds_older_messages_and_walks_back() {
        let h = hist();
        let mut s = HistorySearch::start("draft");
        assert_eq!(s.preview(&h), Some("alpine skiing"), "empty query: newest first");
        for c in "alp".chars() {
            s.push(c);
        }
        assert_eq!(s.preview(&h), Some("alpine skiing"));
        s.older(&h);
        assert_eq!(s.preview(&h), Some("alpha"));
        s.older(&h);
        assert_eq!(s.preview(&h), Some("alpha"), "stays on the oldest match");
        s.push('h');
        assert_eq!(s.preview(&h), Some("alpha"), "typing restarts from the newest match");
        s.push('z');
        assert_eq!(s.preview(&h), None);
        assert_eq!(s.saved_text, "draft");
    }

    #[test]
    fn editor_prefers_visual_and_skips_blank() {
        assert_eq!(editor_command(Some("code -w"), Some("vi")).as_deref(), Some("code -w"));
        assert_eq!(editor_command(Some("  "), Some("vi")).as_deref(), Some("vi"));
        assert_eq!(editor_command(None, None), None);
    }

    #[test]
    fn script_editor_appends_a_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = { let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-unit-tests"); std::fs::create_dir_all(&base).unwrap(); base.join(format!("ed-{}", std::process::id())) };
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ed.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf 'appended line\\n' >> \"$1\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.join("cache").join("draft.md");
        let out = run_external_editor(&script.display().to_string(), &path, "first").unwrap();
        assert_eq!(out, "first\nappended line", "got {out:?}");
        assert!(!path.exists(), "the draft file is removed");
        let fail = run_external_editor("false", &path, "x");
        assert!(fail.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
