//! Plugin dependency resolution and spawn environment injection (`[plugins.<id>]`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use monoagens_reason::SecretString;

use crate::config::PluginConfig;
use crate::runtime::BodyConnectOutcome;

/// Environment variable pointing a spawned plugin to its own self-managed TOML config file.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";

/// Environment variable pointing a spawned plugin to its persistent state directory.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Backward-compatible environment variable for `body-kit` pointing to the plugin state directory.
pub const ENV_BODY_STATE_DIR: &str = "MONOAGENS_BODY_STATE_DIR";

/// Environment variable indicating whether Valve RBAC is enabled (`"1"` or `"0"`).
pub const ENV_RBAC_ENABLED: &str = "MONOAGENS_RBAC_ENABLED";

/// A plugin excluded from booting due to missing/disabled required dependencies or a cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPlugin {
    pub id: String,
    pub reason: String,
}

/// Deterministic topological boot plan produced by [`resolve_plugin_boot_order`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginBootPlan {
    pub ordered: Vec<PluginConfig>,
    pub skipped: Vec<SkippedPlugin>,
}

/// Environment paths, RBAC flags, and plugin secrets injected into spawned stdio plugins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginEnvContext {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub rbac_enabled: bool,
    pub plugin_env: BTreeMap<String, SecretString>,
}

impl PluginEnvContext {
    /// Attaches `[env]` plugin secrets to this environment context.
    #[must_use]
    pub fn with_plugin_env(mut self, plugin_env: BTreeMap<String, SecretString>) -> Self {
        self.plugin_env = plugin_env;
        self
    }
}

/// Summary of connecting all configured plugins in topological dependency order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginStartupReport {
    pub connected: Vec<BodyConnectOutcome>,
    pub failed: Vec<BodyConnectOutcome>,
    pub skipped: Vec<SkippedPlugin>,
}

/// Builds the environment variable pairs injected into a spawned stdio plugin `<id>`.
#[must_use]
pub fn build_plugin_env(plugin_id: &str, ctx: &PluginEnvContext) -> Vec<(String, String)> {
    let mut envs = Vec::new();
    if let Some(config_dir) = &ctx.config_dir {
        let cfg_path = config_dir.join("plugins").join(format!("{plugin_id}.toml"));
        envs.push((
            ENV_PLUGIN_CONFIG.to_owned(),
            cfg_path.display().to_string(),
        ));
    }
    if let Some(data_dir) = &ctx.data_dir {
        let state_dir = data_dir.join("plugins").join(plugin_id);
        let state_str = state_dir.display().to_string();
        envs.push((ENV_PLUGIN_STATE_DIR.to_owned(), state_str.clone()));
        envs.push((ENV_BODY_STATE_DIR.to_owned(), state_str));
    }
    envs.push((
        ENV_RBAC_ENABLED.to_owned(),
        if ctx.rbac_enabled { "1" } else { "0" }.to_owned(),
    ));
    for (key, secret) in &ctx.plugin_env {
        let value = secret.expose();
        if !value.is_empty() && std::env::var_os(key).is_none() {
            envs.push((key.clone(), value.to_owned()));
        }
    }
    envs
}

/// Resolves enabled plugins into a deterministic topological boot order across `dependencies`
/// and satisfied `optional_dependencies`, recognizing `builtin_features` (e.g. `"rbac"`).
///
/// Plugins with `enabled == false` are filtered out cleanly. Any plugin with a missing or
/// disabled required dependency, or participating in a required dependency cycle, is placed
/// into `skipped` (along with dependents that require it) without failing sibling plugins.
#[must_use]
pub fn resolve_plugin_boot_order(
    plugins: &[PluginConfig],
    builtin_features: &[&str],
) -> PluginBootPlan {
    let builtins: BTreeSet<&str> = builtin_features.iter().copied().collect();

    // Filter out disabled plugins and deduplicate by id (preserving first occurrence).
    let mut seen_ids = BTreeSet::new();
    let mut active: Vec<PluginConfig> = Vec::new();
    for plugin in plugins {
        if plugin.enabled && seen_ids.insert(plugin.id.clone()) {
            active.push(plugin.clone());
        }
    }

    let mut active_by_id: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, p) in active.iter().enumerate() {
        active_by_id.insert(p.id.clone(), idx);
    }

    let mut alive = vec![true; active.len()];
    let mut skipped_reasons: BTreeMap<usize, String> = BTreeMap::new();

    // 1. Iteratively prune plugins with missing, disabled, or failed required dependencies.
    loop {
        let mut changed = false;
        for (idx, plugin) in active.iter().enumerate() {
            if !alive[idx] {
                continue;
            }
            let mut missing_dep: Option<String> = None;
            for dep in &plugin.dependencies {
                if builtins.contains(dep.as_str()) {
                    continue;
                }
                match active_by_id.get(dep) {
                    Some(&dep_idx) if alive[dep_idx] => {}
                    _ => {
                        missing_dep = Some(dep.clone());
                        break;
                    }
                }
            }
            if let Some(dep) = missing_dep {
                alive[idx] = false;
                skipped_reasons.insert(
                    idx,
                    format!("missing or disabled required dependency '{dep}'"),
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 2. Detect cycles (and dependents of cycles) over required dependencies using Kahn's algorithm.
    let mut req_indegree = vec![0usize; active.len()];
    let mut req_dependents: Vec<Vec<usize>> = vec![Vec::new(); active.len()];
    for (idx, plugin) in active.iter().enumerate() {
        if !alive[idx] {
            continue;
        }
        let mut unique_deps = BTreeSet::new();
        for dep in &plugin.dependencies {
            if builtins.contains(dep.as_str()) {
                continue;
            }
            if let Some(&dep_idx) = active_by_id.get(dep)
                && alive[dep_idx]
                && unique_deps.insert(dep_idx)
            {
                req_dependents[dep_idx].push(idx);
                req_indegree[idx] += 1;
            }
        }
    }

    let mut ready_req = BTreeSet::new();
    for (idx, &deg) in req_indegree.iter().enumerate() {
        if alive[idx] && deg == 0 {
            ready_req.insert(idx);
        }
    }

    let mut acyclic_alive = vec![false; active.len()];
    while let Some(u) = ready_req.pop_first() {
        acyclic_alive[u] = true;
        for &v in &req_dependents[u] {
            req_indegree[v] -= 1;
            if req_indegree[v] == 0 {
                ready_req.insert(v);
            }
        }
    }

    for idx in 0..active.len() {
        if alive[idx] && !acyclic_alive[idx] {
            alive[idx] = false;
            skipped_reasons.insert(idx, "participates in or requires a dependency cycle".to_owned());
        }
    }

    // 3. Topologically sort surviving plugins over both required `dependencies` and satisfied
    //    `optional_dependencies`. Optional edges never cause valid plugins to be skipped if an
    //    optional dependency creates a mutual ordering tie.
    let mut req_in = vec![0usize; active.len()];
    let mut opt_in = vec![0usize; active.len()];
    let mut req_edges: Vec<Vec<usize>> = vec![Vec::new(); active.len()];
    let mut opt_edges: Vec<Vec<usize>> = vec![Vec::new(); active.len()];

    for (idx, plugin) in active.iter().enumerate() {
        if !alive[idx] {
            continue;
        }
        let mut req_set = BTreeSet::new();
        for dep in &plugin.dependencies {
            if builtins.contains(dep.as_str()) {
                continue;
            }
            if let Some(&dep_idx) = active_by_id.get(dep)
                && alive[dep_idx]
                && req_set.insert(dep_idx)
            {
                req_edges[dep_idx].push(idx);
                req_in[idx] += 1;
            }
        }
        let mut opt_set = BTreeSet::new();
        for dep in &plugin.optional_dependencies {
            if builtins.contains(dep.as_str()) {
                continue;
            }
            if let Some(&dep_idx) = active_by_id.get(dep)
                && alive[dep_idx]
                && dep_idx != idx
                && !req_set.contains(&dep_idx)
                && opt_set.insert(dep_idx)
            {
                opt_edges[dep_idx].push(idx);
                opt_in[idx] += 1;
            }
        }
    }

    let mut placed = vec![false; active.len()];
    let mut ordered = Vec::new();
    let remaining_total: usize = alive.iter().filter(|&&a| a).count();

    while ordered.len() < remaining_total {
        // Prefer nodes with req_in == 0 and opt_in == 0.
        let next_idx = (0..active.len())
            .find(|&i| alive[i] && !placed[i] && req_in[i] == 0 && opt_in[i] == 0)
            // If an optional-dependency cycle stalls progress, break the optional tie cleanly
            // by picking the earliest node whose required dependencies are all satisfied.
            .or_else(|| (0..active.len()).find(|&i| alive[i] && !placed[i] && req_in[i] == 0));

        let Some(u) = next_idx else {
            break;
        };
        placed[u] = true;
        ordered.push(active[u].clone());
        for &v in &req_edges[u] {
            req_in[v] = req_in[v].saturating_sub(1);
        }
        for &v in &opt_edges[u] {
            opt_in[v] = opt_in[v].saturating_sub(1);
        }
    }

    let skipped = skipped_reasons
        .into_iter()
        .map(|(idx, reason)| SkippedPlugin {
            id: active[idx].id.clone(),
            reason,
        })
        .collect();

    PluginBootPlan { ordered, skipped }
}
