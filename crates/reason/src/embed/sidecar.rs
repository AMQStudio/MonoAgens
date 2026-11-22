//! Shared Python interpreter resolution and lifecycle management for the local `EmbeddingGemma 2` HTTP sidecar.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Environment variable overriding the Python interpreter used for local adapter sidecars.
pub const ENV_ADAPTER_PYTHON: &str = "MONOAGENS_ADAPTER_PYTHON";

/// Resolves the Python executable path for local adapter/sidecar processes.
#[must_use]
pub fn resolve_adapter_python(workspace_root: &Path) -> String {
    resolve_adapter_python_with(workspace_root, |key| std::env::var(key).ok())
}

fn resolve_adapter_python_with<F>(workspace_root: &Path, env_lookup: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(py) = env_lookup(ENV_ADAPTER_PYTHON) {
        let trimmed = py.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    for cand in [
        workspace_root.join("lab/adapters/.venv/bin/python"),
        workspace_root.join("training/.venv/bin/python"),
    ] {
        if cand.is_file() {
            return cand.to_string_lossy().into_owned();
        }
    }
    "python3".to_owned()
}

/// Managed local `EmbeddingGemma 2` HTTP sidecar child process.
#[derive(Debug)]
pub struct LocalEmbedSidecar {
    child: Child,
    url: String,
    device_info: String,
}

impl LocalEmbedSidecar {
    /// Spawns the sidecar script with `--model-path <model_dir> --port 0` and waits for its `READY <url> [<device_info>]` line.
    pub fn start(script: &Path, model_dir: &Path, workspace_root: &Path) -> Result<Self, String> {
        if !script.is_file() {
            return Err(format!(
                "embeddinggemma2_http sidecar script not found at '{}'",
                script.display()
            ));
        }
        if !model_dir.is_dir() {
            return Err(format!(
                "embeddinggemma2_http requires model_path pointing to an existing directory (missing '{}')",
                model_dir.display()
            ));
        }

        let py = resolve_adapter_python(workspace_root);
        let mut child = Command::new(&py)
            .arg(script)
            .arg("--model-path")
            .arg(model_dir)
            .arg("--port")
            .arg("0")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn EmbeddingGemma 2 sidecar via '{py}': {e}"))?;

        let Some(stdout) = child.stdout.take() else {
            let stderr_msg = kill_and_collect_stderr(child);
            return Err(format!(
                "EmbeddingGemma 2 sidecar stdout unavailable (stderr='{}')",
                stderr_msg.trim()
            ));
        };
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        if let Err(e) = reader.read_line(&mut first_line) {
            drop(reader);
            let stderr_msg = kill_and_collect_stderr(child);
            return Err(format!(
                "failed reading EmbeddingGemma 2 sidecar readiness line: {e} (stderr='{}')",
                stderr_msg.trim()
            ));
        }

        let trimmed = first_line.trim();
        let Some(rest) = trimmed.strip_prefix("READY ") else {
            drop(reader);
            let stderr_msg = kill_and_collect_stderr(child);
            return Err(format!(
                "EmbeddingGemma 2 sidecar failed to start (stdout='{}', stderr='{}')",
                trimmed,
                stderr_msg.trim()
            ));
        };

        let mut parts = rest.splitn(2, ' ');
        let url = parts.next().unwrap_or("").trim().to_owned();
        let device_info = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_owned();
        if url.is_empty() {
            drop(reader);
            let stderr_msg = kill_and_collect_stderr(child);
            return Err(format!(
                "EmbeddingGemma 2 sidecar returned empty URL (stdout='{}', stderr='{}')",
                trimmed,
                stderr_msg.trim()
            ));
        }

        Ok(Self {
            child,
            url,
            device_info,
        })
    }

    /// Returns the sidecar HTTP base URL reported on startup.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the execution device label reported by the sidecar (`"unknown"` if omitted).
    #[must_use]
    pub fn device_info(&self) -> &str {
        &self.device_info
    }
}

impl Drop for LocalEmbedSidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn kill_and_collect_stderr(mut child: Child) -> String {
    let _ = child.kill();
    child
        .wait_with_output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn resolve_adapter_python_respects_env_and_venv_precedence() {
        let ws = tempfile::tempdir().expect("tempdir");

        // 1. Non-empty env var wins over everything.
        let from_env = resolve_adapter_python_with(ws.path(), |k| {
            (k == ENV_ADAPTER_PYTHON).then(|| "  /custom/python3  ".to_owned())
        });
        assert_eq!(from_env, "/custom/python3");

        // 2. Blank env var is ignored and falls back to python3 when no venv exists.
        let fallback =
            resolve_adapter_python_with(ws.path(), |k| (k == ENV_ADAPTER_PYTHON).then(|| "   ".to_owned()));
        assert_eq!(fallback, "python3");

        // 3. training/.venv/bin/python used if lab/adapters/.venv/bin/python is absent.
        let training_py = ws.path().join("training/.venv/bin/python");
        fs::create_dir_all(training_py.parent().expect("parent")).expect("mkdir");
        fs::write(&training_py, "").expect("write training python");
        assert_eq!(
            resolve_adapter_python_with(ws.path(), |_| None),
            training_py.to_string_lossy()
        );

        // 4. lab/adapters/.venv/bin/python takes precedence over training/.venv/bin/python.
        let adapters_py = ws.path().join("lab/adapters/.venv/bin/python");
        fs::create_dir_all(adapters_py.parent().expect("parent")).expect("mkdir");
        fs::write(&adapters_py, "").expect("write adapters python");
        assert_eq!(
            resolve_adapter_python_with(ws.path(), |_| None),
            adapters_py.to_string_lossy()
        );

        // Public wrapper falls back to python3 for an empty workspace when env var is unset.
        let empty_ws = tempfile::tempdir().expect("empty tempdir");
        if std::env::var(ENV_ADAPTER_PYTHON)
            .ok()
            .is_none_or(|v| v.trim().is_empty())
        {
            assert_eq!(resolve_adapter_python(empty_ws.path()), "python3");
        }
    }

    #[test]
    fn local_embed_sidecar_validates_paths_and_parses_readiness() {
        let ws = tempfile::tempdir().expect("tempdir");
        let missing_script = ws.path().join("missing.py");
        let model_dir = ws.path().join("model");
        fs::create_dir_all(&model_dir).expect("mkdir model");

        let err = LocalEmbedSidecar::start(&missing_script, &model_dir, ws.path())
            .expect_err("missing script must fail");
        assert!(err.contains("sidecar script not found"), "unexpected err: {err}");

        let script = ws.path().join("mock_sidecar.py");
        fs::write(&script, "print('READY http://127.0.0.1:0 cpu')\n").expect("write script");
        let missing_model = ws.path().join("no_such_model");
        let err = LocalEmbedSidecar::start(&script, &missing_model, ws.path())
            .expect_err("missing model_dir must fail");
        assert!(err.contains("existing directory"), "unexpected err: {err}");

        // Success with explicit device_info.
        fs::write(
            &script,
            "import sys, time\nassert '--model-path' in sys.argv and '--port' in sys.argv\nprint('READY http://127.0.0.1:43210 cuda:0', flush=True)\ntime.sleep(5)\n",
        )
        .expect("write ready script");
        let sidecar =
            LocalEmbedSidecar::start(&script, &model_dir, ws.path()).expect("sidecar should start");
        assert_eq!(sidecar.url(), "http://127.0.0.1:43210");
        assert_eq!(sidecar.device_info(), "cuda:0");
        drop(sidecar);

        // Success with missing device_info defaults to "unknown".
        fs::write(
            &script,
            "import time\nprint('READY http://127.0.0.1:43211', flush=True)\ntime.sleep(5)\n",
        )
        .expect("write ready without device");
        let sidecar_no_dev =
            LocalEmbedSidecar::start(&script, &model_dir, ws.path()).expect("sidecar should start");
        assert_eq!(sidecar_no_dev.url(), "http://127.0.0.1:43211");
        assert_eq!(sidecar_no_dev.device_info(), "unknown");
        drop(sidecar_no_dev);

        // Failure on non-READY stdout captures both stdout and stderr.
        fs::write(
            &script,
            "import sys\nprint('BOOT_ERR', flush=True)\nprint('boom detail', file=sys.stderr, flush=True)\n",
        )
        .expect("write failing script");
        let err = LocalEmbedSidecar::start(&script, &model_dir, ws.path())
            .expect_err("bad readiness line must fail");
        assert!(err.contains("BOOT_ERR"), "missing stdout in err: {err}");
        assert!(err.contains("boom detail"), "missing stderr in err: {err}");

        // Failure on empty URL after READY prefix.
        fs::write(
            &script,
            "import sys\nprint('READY  cpu', flush=True)\nprint('empty url stderr', file=sys.stderr, flush=True)\n",
        )
        .expect("write empty-url script");
        let err = LocalEmbedSidecar::start(&script, &model_dir, ws.path())
            .expect_err("empty URL must fail");
        assert!(err.contains("empty URL"), "unexpected err: {err}");
        assert!(err.contains("empty url stderr"), "missing stderr in err: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn local_embed_sidecar_uses_workspace_venv_python_when_present() {
        let ws = tempfile::tempdir().expect("tempdir");
        if std::env::var(ENV_ADAPTER_PYTHON)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        {
            return;
        }
        let venv_py = ws.path().join("lab/adapters/.venv/bin/python");
        fs::create_dir_all(venv_py.parent().expect("mkdir")).expect("mkdir");
        fs::write(
            &venv_py,
            "#!/bin/sh\necho 'READY http://127.0.0.1:45555 venv-mock'\nsleep 2\n",
        )
        .expect("write mock venv python");
        let mut perms = fs::metadata(&venv_py).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&venv_py, perms).expect("chmod");

        let script = ws.path().join("dummy.py");
        fs::write(&script, "").expect("write dummy script");
        let model_dir = ws.path().join("model");
        fs::create_dir_all(&model_dir).expect("mkdir model");

        let sidecar =
            LocalEmbedSidecar::start(&script, &model_dir, ws.path()).expect("start via venv");
        assert_eq!(sidecar.url(), "http://127.0.0.1:45555");
        assert_eq!(sidecar.device_info(), "venv-mock");
    }
}
