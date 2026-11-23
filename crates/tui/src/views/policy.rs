//! `/policy`: read-only view of valve toggles, the two absolute lines, active Cedar rules and
//! the RBAC summary, from `policy.get` (the data `monoagens inspector` prints). No edit keys.

use crossterm::event::{KeyCode, KeyEvent};
use serde_json::{Value, json};

use super::{Detail, GovRequest, GovTag, Nav, Row, Tone, ViewModel, jstr};
use crate::state::AppAction;

#[derive(Debug, Clone, Default)]
pub struct PolicyView {
    pub report: Option<Value>,
    /// `(label, on, description)` of the valve rule toggles (from the configuration file).
    pub toggles: Vec<(String, bool, String)>,
    pub nav: Nav,
    pub error: Option<String>,
}

#[must_use]
pub fn request() -> GovRequest {
    GovRequest::new(GovTag::Policy, "policy.get", &json!({}))
}

/// What a list line stands for.
enum Line {
    Plain,
    Rule(usize),
}

/// The valve rule toggles with the descriptions `/settings -> Valve` shows.
#[must_use]
pub fn toggle_rows(t: &crate::settings::ValveRuleTogglesState) -> Vec<(String, bool, String)> {
    [
        ("cedar_abac", t.cedar_abac, "Cedar policy ABAC evaluation"),
        ("line2_scope", t.line2_scope, "VisibleScope Line-2 audience boundary"),
        ("taint_blocking", t.taint_blocking, "3-door taint quarantine enforcement"),
        ("pending_approval", t.pending_approval, "Hold high-risk actions for owner approval"),
        ("budget_guard", t.budget_guard, "Enforce daily_token_budget"),
        ("chain_brake", t.chain_brake, "Enforce max_chain_wakes circuit breaker"),
        ("quiet_hours", t.quiet_hours, "Defer non-urgent outbound notifications in quiet hours"),
        ("proactive_cap", t.proactive_cap, "Daily cap on unsolicited proactive notifications"),
    ]
    .into_iter()
    .map(|(l, on, d)| (l.to_owned(), on, d.to_owned()))
    .collect()
}

impl PolicyView {
    pub fn set(&mut self, v: &Value) {
        self.report = Some(v.clone());
        let n = self.lines().len();
        self.nav.clamp(n);
    }

    fn lines(&self) -> Vec<(Row, Line)> {
        let mut out: Vec<(Row, Line)> = Vec::new();
        let Some(r) = &self.report else {
            return out;
        };
        let plain = |t: String, tone: Tone| (Row::new(t, tone), Line::Plain);
        out.push(plain(format!("Valve mode: {}", jstr(r, "valve_mode")), Tone::Head));
        let proof = r.get("symcc_proof").cloned().unwrap_or(Value::Null);
        out.push(plain(
            format!(
                "  proof: {}  solver {}  counterexamples {}",
                proof.get("status").map_or("-".to_owned(), |s| s.as_str().map_or(s.to_string(), str::to_owned)),
                jstr(&proof, "solver"),
                proof.get("counterexamples").and_then(Value::as_u64).unwrap_or(0)
            ),
            Tone::Dim,
        ));
        out.push(plain("Rule toggles".to_owned(), Tone::Head));
        for (label, on, desc) in &self.toggles {
            out.push(plain(
                format!("  [{}] {label:<18} {desc}", if *on { "on " } else { "off" }),
                if *on { Tone::Good } else { Tone::Warn },
            ));
        }
        out.push(plain("Absolute lines".to_owned(), Tone::Head));
        for (i, l) in r.get("absolute_lines").and_then(Value::as_array).into_iter().flatten().enumerate() {
            out.push(plain(format!("  {}. {}", i + 1, l.as_str().unwrap_or_default()), Tone::Normal));
        }
        let rules = r.get("rules").and_then(Value::as_array).cloned().unwrap_or_default();
        out.push(plain(format!("Cedar rules ({})  Enter shows the source", rules.len()), Tone::Head));
        for (i, rule) in rules.iter().enumerate() {
            let tone = if jstr(rule, "kind") == "forbid" { Tone::Bad } else { Tone::Normal };
            out.push((Row::new(format!("  {}", jstr(rule, "id")), tone), Line::Rule(i)));
        }
        out.push(plain("RBAC".to_owned(), Tone::Head));
        let rbac = r.get("rbac").cloned().unwrap_or(Value::Null);
        out.push(plain(
            format!(
                "  fallback role: {}   place rules: {}",
                rbac.get("fallback_role").and_then(Value::as_str).unwrap_or("(none)"),
                rbac.get("place_rules").and_then(Value::as_u64).unwrap_or(0)
            ),
            Tone::Normal,
        ));
        let roles: Vec<String> = rbac
            .get("roles")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        out.push(plain(format!("  roles: {}", if roles.is_empty() { "(none)".to_owned() } else { roles.join(", ") }), Tone::Normal));
        for pair in rbac.get("actor_roles").and_then(Value::as_array).into_iter().flatten() {
            let actor = pair.get(0).and_then(Value::as_str).unwrap_or_default();
            let rs: Vec<&str> = pair
                .get(1)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            out.push(plain(format!("  {actor} -> {}", rs.join(", ")), Tone::Dim));
        }
        out
    }

    pub fn handle_key(&mut self, key: KeyEvent, visible: usize, detail: &mut Option<Detail>) -> Vec<AppAction> {
        let lines = self.lines();
        if self.nav.handle_key(key, lines.len(), visible) {
            return Vec::new();
        }
        if key.code == KeyCode::Enter
            && let Some((_, Line::Rule(i))) = lines.get(self.nav.selected)
            && let Some(rule) = self
                .report
                .as_ref()
                .and_then(|r| r.get("rules"))
                .and_then(Value::as_array)
                .and_then(|a| a.get(*i))
        {
            *detail = Some(Detail {
                title: format!("Rule {}", jstr(rule, "id")),
                lines: jstr(rule, "source").lines().map(str::to_owned).collect(),
                offset: 0,
            });
        }
        Vec::new()
    }

    #[must_use]
    pub fn view(&self) -> ViewModel {
        let lines = self.lines();
        ViewModel {
            title: "Policy (read-only)".to_owned(),
            header: String::new(),
            rows: lines.into_iter().map(|(r, _)| r).collect(),
            selected: Some(self.nav.selected),
            offset: self.nav.offset,
            footer: " Enter rule source  Up/Down PgUp/PgDn  Esc close".to_owned(),
            empty: self.error.clone().unwrap_or_else(|| "Loading...".to_owned()),
        }
    }
}
