//! `/people`: persons with their platform accounts and roles, active links; initiate, verify
//! and unlink (owner only; the gateway refuses anyone else). Same data and behaviour as
//! `monoagens link`.

use crossterm::event::{KeyCode, KeyEvent};
use serde_json::{Value, json};

use super::{Detail, GovRequest, GovTag, Nav, Prompt, PromptFor, Row, Tone, ViewModel, jstr};
use crate::state::AppAction;

#[derive(Debug, Clone)]
pub struct Person {
    pub actor_id: String,
    pub is_owner: bool,
    pub roles: Vec<String>,
    pub accounts: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct LinkRow {
    pub body_id: String,
    pub account_id: String,
    pub to_actor: String,
    pub owner_confirmed: bool,
}

/// What a list line stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Person,
    Account(String, String),
    Link(String, String),
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct PeopleView {
    pub persons: Vec<Person>,
    pub links: Vec<LinkRow>,
    pub fallback_role: Option<String>,
    pub nav: Nav,
    /// Pending unlink confirmation: `(body, account)`.
    pub confirm: Option<(String, String)>,
    pub loaded: bool,
}

#[must_use]
pub fn request() -> GovRequest {
    GovRequest::new(GovTag::People, "people.list", &json!({}))
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// `links.initiate` from the typed text: with a preselected primary account the text is
/// `secondary_body secondary_account`, otherwise all four words.
pub fn initiate_request(primary: Option<(String, String)>, text: &str) -> Result<GovRequest, String> {
    let w = words(text);
    let (p, s) = match (primary, w.as_slice()) {
        (Some(p), [sb, sa]) => (p, ((*sb).to_owned(), (*sa).to_owned())),
        (None, [pb, pa, sb, sa]) => (((*pb).to_owned(), (*pa).to_owned()), ((*sb).to_owned(), (*sa).to_owned())),
        (Some(_), _) => return Err("Enter: <secondary body> <secondary account>".to_owned()),
        (None, _) => return Err("Enter: <primary body> <primary account> <secondary body> <secondary account>".to_owned()),
    };
    Ok(GovRequest::new(
        GovTag::LinkInitiate,
        "links.initiate",
        &json!({
            "primary_body": p.0, "primary_account": p.1,
            "secondary_body": s.0, "secondary_account": s.1,
            "emit_nonces": true,
        }),
    ))
}

/// `links.verify` from `<challenge> <nonce1> <nonce2> [owner]`.
pub fn verify_request(text: &str) -> Result<GovRequest, String> {
    let w = words(text);
    if w.len() < 3 || w.len() > 4 || (w.len() == 4 && w[3] != "owner") {
        return Err("Enter: <challenge id> <nonce 1> <nonce 2> [owner]".to_owned());
    }
    Ok(GovRequest::new(
        GovTag::LinkVerify,
        "links.verify",
        &json!({ "challenge": w[0], "nonce1": w[1], "nonce2": w[2], "owner_confirmed": w.len() == 4 }),
    ))
}

#[must_use]
pub fn initiate_instructions(v: &Value) -> Detail {
    let mut lines = vec![
        format!("challenge id: {}", jstr(v, "challenge_id")),
        format!("primary actor: {}", jstr(v, "primary_actor")),
        format!("secondary actor: {}", jstr(v, "secondary_actor")),
        format!("expires at (ms): {}", v.get("expires_at_ms").and_then(Value::as_u64).unwrap_or(0)),
    ];
    if let Some(n) = v.get("nonce_1").and_then(Value::as_str) {
        lines.push(format!("nonce 1 (send to the primary account): {n}"));
    }
    if let Some(n) = v.get("nonce_2").and_then(Value::as_str) {
        lines.push(format!("nonce 2 (send to the secondary account): {n}"));
    }
    lines.push(String::new());
    lines.push("Deliver each nonce out of band, then press v and enter:".to_owned());
    lines.push("  <challenge id> <nonce 1> <nonce 2> [owner]".to_owned());
    Detail { title: "Link challenge issued".to_owned(), lines, offset: 0 }
}

#[must_use]
pub fn link_result_line(v: &Value) -> String {
    match v.get("status").and_then(Value::as_str) {
        Some("linked") => format!("linked actor={}", jstr(v, "actor_id")),
        Some("unlinked") => format!("unlinked ({}, {})", jstr(v, "body_id"), jstr(v, "account_id")),
        _ => v.to_string(),
    }
}

impl PeopleView {
    pub fn set(&mut self, v: &Value) {
        self.persons = v
            .get("configured_persons")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| Person {
                        actor_id: jstr(p, "actor_id"),
                        is_owner: p.get("is_owner").and_then(Value::as_bool).unwrap_or(false),
                        roles: p
                            .get("roles")
                            .and_then(Value::as_array)
                            .map(|r| r.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                            .unwrap_or_default(),
                        accounts: p
                            .get("accounts")
                            .and_then(Value::as_array)
                            .map(|r| r.iter().map(|x| (jstr(x, "body_id"), jstr(x, "account_id"))).collect())
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.links = v
            .get("active_links")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|l| LinkRow {
                        body_id: jstr(l, "body_id"),
                        account_id: jstr(l, "account_id"),
                        to_actor: jstr(l, "to_actor"),
                        owner_confirmed: l.get("owner_confirmed").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.fallback_role = v.get("fallback_role").and_then(Value::as_str).map(str::to_owned);
        self.loaded = true;
        let total = self.items().len();
        self.nav.clamp(total);
    }

    fn items(&self) -> Vec<(Row, Item)> {
        let mut out = Vec::new();
        for p in &self.persons {
            let roles = if p.roles.is_empty() {
                self.fallback_role.as_ref().map_or_else(|| "no role".to_owned(), |f| format!("{f} (fallback)"))
            } else {
                p.roles.join(", ")
            };
            out.push((
                Row::new(
                    format!("{}{}   roles: {roles}", p.actor_id, if p.is_owner { "  [owner]" } else { "" }),
                    Tone::Head,
                ),
                Item::Person,
            ));
            for (b, a) in &p.accounts {
                out.push((Row::new(format!("    {b}  {a}"), Tone::Normal), Item::Account(b.clone(), a.clone())));
            }
        }
        if !self.links.is_empty() {
            out.push((Row::new("Active links", Tone::Head), Item::Other));
        }
        for l in &self.links {
            out.push((
                Row::new(
                    format!(
                        "    {}  {}  ->  {}{}",
                        l.body_id,
                        l.account_id,
                        l.to_actor,
                        if l.owner_confirmed { "  (owner confirmed)" } else { "" }
                    ),
                    Tone::Good,
                ),
                Item::Link(l.body_id.clone(), l.account_id.clone()),
            ));
        }
        out
    }

    pub fn cancel_confirm(&mut self) -> bool {
        self.confirm.take().is_some()
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        visible: usize,
        prompt: &mut Option<Prompt>,
        _detail: &mut Option<Detail>,
    ) -> Vec<AppAction> {
        if let Some((body, account)) = self.confirm.clone() {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.confirm = None;
                    return vec![AppAction::Gov(GovRequest::new(
                        GovTag::LinkUnlink,
                        "links.unlink",
                        &json!({ "body": body, "account": account, "reason": "unlinked from the TUI" }),
                    ))];
                }
                KeyCode::Char('n' | 'N') => self.confirm = None,
                _ => {}
            }
            return Vec::new();
        }
        let items = self.items();
        if self.nav.handle_key(key, items.len(), visible) {
            return Vec::new();
        }
        let cur = items.get(self.nav.selected).map(|(_, i)| i.clone());
        match key.code {
            KeyCode::Char('l') => {
                let primary = match cur {
                    Some(Item::Account(b, a)) => Some((b, a)),
                    _ => None,
                };
                let label = if primary.is_some() {
                    "link to (body account)"
                } else {
                    "link (primary body account, secondary body account)"
                };
                *prompt = Some(Prompt {
                    label: label.to_owned(),
                    text: String::new(),
                    purpose: PromptFor::LinkInitiate { primary },
                });
            }
            KeyCode::Char('v') => {
                *prompt = Some(Prompt {
                    label: "verify (challenge nonce1 nonce2 [owner])".to_owned(),
                    text: String::new(),
                    purpose: PromptFor::LinkVerify,
                });
            }
            KeyCode::Char('u') => {
                if let Some(Item::Account(b, a) | Item::Link(b, a)) = cur {
                    self.confirm = Some((b, a));
                }
            }
            KeyCode::Char('r') => return vec![AppAction::Gov(request())],
            _ => {}
        }
        Vec::new()
    }

    #[must_use]
    pub fn view(&self) -> ViewModel {
        let items = self.items();
        let footer = match &self.confirm {
            Some((b, a)) => format!(" Unlink {b} {a}?  y confirm  n cancel"),
            None => " l link  v verify  u unlink  r refresh  Esc close".to_owned(),
        };
        ViewModel {
            title: "People & links".to_owned(),
            header: format!("{} persons, {} active links", self.persons.len(), self.links.len()),
            rows: items.into_iter().map(|(r, _)| r).collect(),
            selected: Some(self.nav.selected),
            offset: self.nav.offset,
            footer,
            empty: if self.loaded { "No persons configured.".to_owned() } else { "Loading...".to_owned() },
        }
    }
}
