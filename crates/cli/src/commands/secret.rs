use std::io::{self, BufRead, Write};
use std::path::Path;

use monoagens_gateway::resolve_paths;
use serde_json::json;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::common::write_json_pretty;
use crate::{SecretArgs, SecretSubcommand};

fn validate_secret_key(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("secret key must not be empty".to_owned());
    }
    if trimmed.contains(char::is_whitespace) || trimmed.contains('.') {
        return Err(format!(
            "invalid secret key '{trimmed}' (must not contain whitespace or '.')"
        ));
    }
    Ok(trimmed.to_owned())
}

fn read_or_init_secrets_doc(path: &Path) -> Result<DocumentMut, String> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read secrets file '{}': {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    if raw.trim_start().starts_with('{') {
        return Err(format!(
            "secrets file '{}' is JSON-formatted; convert it to TOML to manage keys with `monoagens secret`",
            path.display()
        ));
    }
    raw.parse::<DocumentMut>()
        .map_err(|e| format!("invalid TOML in secrets file '{}': {e}", path.display()))
}

fn write_secrets_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create directory '{}' for secrets file: {e}",
                parent.display()
            )
        })?;
    }
    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("failed to write secrets file '{}': {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            format!(
                "failed to set 0600 permissions on '{}': {e}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn collect_non_env_keys(table: &Table, prefix: &str, out: &mut Vec<String>) {
    for (k, item) in table.iter() {
        if prefix.is_empty() && k == "env" {
            continue;
        }
        let full_key = if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        };
        match item {
            Item::Table(sub) => collect_non_env_keys(sub, &full_key, out),
            Item::Value(toml_edit::Value::InlineTable(it)) => {
                for (ik, _) in it.iter() {
                    out.push(format!("{full_key}.{ik}"));
                }
            }
            Item::None => {}
            _ => out.push(full_key),
        }
    }
}

fn cmd_secret_list(
    secrets_path: &Path,
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    if !secrets_path.exists() {
        if json_mode {
            write_json_pretty(
                out,
                &json!({
                    "path": secrets_path.display().to_string(),
                    "exists": false,
                    "keys": Vec::<String>::new(),
                    "env_keys": Vec::<String>::new(),
                }),
            )?;
        } else {
            writeln!(
                out,
                "no secrets file at {} (0 keys)",
                secrets_path.display()
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(0);
    }

    let doc = read_or_init_secrets_doc(secrets_path)?;
    let mut keys = Vec::new();
    collect_non_env_keys(doc.as_table(), "", &mut keys);

    let mut env_keys = Vec::new();
    if let Some(env_item) = doc.get("env")
        && let Some(env_tbl) = env_item.as_table_like()
    {
        for (k, item) in env_tbl.iter() {
            if !item.is_none() {
                env_keys.push(k.to_owned());
            }
        }
    }

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "path": secrets_path.display().to_string(),
                "exists": true,
                "keys": keys,
                "env_keys": env_keys,
            }),
        )?;
        return Ok(0);
    }

    writeln!(out, "secrets file: {}", secrets_path.display()).map_err(|e| e.to_string())?;
    if keys.is_empty() {
        writeln!(out, "top-level keys: (none)").map_err(|e| e.to_string())?;
    } else {
        writeln!(out, "top-level keys ({}):", keys.len()).map_err(|e| e.to_string())?;
        for k in &keys {
            writeln!(out, "  - {k}").map_err(|e| e.to_string())?;
        }
    }
    if env_keys.is_empty() {
        writeln!(out, "[env] keys: (none)").map_err(|e| e.to_string())?;
    } else {
        writeln!(out, "[env] keys ({}):", env_keys.len()).map_err(|e| e.to_string())?;
        for k in &env_keys {
            writeln!(out, "  - {k}").map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn resolve_secret_input_value(
    inline_value: Option<String>,
    from_env: Option<String>,
    from_stdin: bool,
    stdin_reader: &mut dyn BufRead,
) -> Result<String, String> {
    let raw = if let Some(v) = inline_value {
        v
    } else if let Some(var_name) = from_env {
        let name = var_name.trim();
        if name.is_empty() {
            return Err("--from-env variable name must not be empty".to_owned());
        }
        std::env::var(name)
            .map_err(|_| format!("environment variable '{name}' is not set"))?
    } else if from_stdin {
        let mut line = String::new();
        stdin_reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read secret value from stdin: {e}"))?;
        line
    } else {
        return Err("must specify one of --value <VALUE>, --from-env <VAR>, or --stdin".to_owned());
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("secret value must not be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

struct SecretSetParams {
    key: String,
    value: Option<String>,
    from_env: Option<String>,
    stdin: bool,
    env_scope: bool,
}

fn cmd_secret_set(
    secrets_path: &Path,
    json_mode: bool,
    params: SecretSetParams,
    stdin_reader: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let key = validate_secret_key(&params.key)?;
    let secret_val = resolve_secret_input_value(
        params.value,
        params.from_env,
        params.stdin,
        stdin_reader,
    )?;

    let mut doc = read_or_init_secrets_doc(secrets_path)?;
    if params.env_scope {
        let env_item = doc.entry("env").or_insert_with(|| Item::Table(Table::new()));
        let env_tbl = env_item
            .as_table_like_mut()
            .ok_or_else(|| "secret section '[env]' must be a TOML table".to_owned())?;
        env_tbl.insert(&key, value(secret_val));
    } else {
        if key == "env" {
            return Err("top-level key 'env' is reserved for the [env] plugin table; pass --env to set variables under [env]".to_owned());
        }
        doc[&key] = value(secret_val);
    }

    write_secrets_doc(secrets_path, &doc)?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "action": "set",
                "scope": if params.env_scope { "env" } else { "top_level" },
                "key": key,
                "path": secrets_path.display().to_string(),
            }),
        )?;
    } else if params.env_scope {
        writeln!(
            out,
            "set [env] secret '{}' in {}",
            key,
            secrets_path.display()
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(out, "set secret '{}' in {}", key, secrets_path.display())
            .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn cmd_secret_unset(
    secrets_path: &Path,
    json_mode: bool,
    raw_key: String,
    env_scope: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let key = validate_secret_key(&raw_key)?;
    if !secrets_path.exists() {
        return Err(format!(
            "secret '{}' not found (secrets file '{}' does not exist)",
            key,
            secrets_path.display()
        ));
    }

    let mut doc = read_or_init_secrets_doc(secrets_path)?;
    let removed = if env_scope {
        let Some(env_tbl) = doc.get_mut("env").and_then(Item::as_table_like_mut) else {
            return Err(format!(
                "[env] secret '{}' not found in '{}'",
                key,
                secrets_path.display()
            ));
        };
        let prev = env_tbl.remove(&key).is_some();
        if env_tbl.is_empty() {
            doc.as_table_mut().remove("env");
        }
        prev
    } else {
        doc.as_table_mut().remove(&key).is_some()
    };

    if !removed {
        return Err(format!(
            "{}secret '{}' not found in '{}'",
            if env_scope { "[env] " } else { "" },
            key,
            secrets_path.display()
        ));
    }

    write_secrets_doc(secrets_path, &doc)?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "action": "unset",
                "scope": if env_scope { "env" } else { "top_level" },
                "key": key,
                "path": secrets_path.display().to_string(),
            }),
        )?;
    } else if env_scope {
        writeln!(
            out,
            "unset [env] secret '{}' in {}",
            key,
            secrets_path.display()
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(out, "unset secret '{}' in {}", key, secrets_path.display())
            .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

pub(crate) fn cmd_secret(
    config_path: Option<&Path>,
    json_mode: bool,
    args: SecretArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let paths = resolve_paths(config_path).map_err(|e| e.to_string())?;
    let mut stdin = io::stdin().lock();
    match args.action {
        SecretSubcommand::List => cmd_secret_list(&paths.secrets_path, json_mode, out),
        SecretSubcommand::Set {
            key,
            value,
            from_env,
            stdin: use_stdin,
            env,
        } => cmd_secret_set(
            &paths.secrets_path,
            json_mode,
            SecretSetParams {
                key,
                value,
                from_env,
                stdin: use_stdin,
                env_scope: env,
            },
            &mut stdin,
            out,
        ),
        SecretSubcommand::Unset { key, env } => {
            cmd_secret_unset(&paths.secrets_path, json_mode, key, env, out)
        }
    }
}
