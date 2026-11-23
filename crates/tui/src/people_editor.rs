//! Pure edit logic of the `/people` editor: persons, their platform accounts and roles, and the
//! `config.apply` patch that writes them to `[[person]]`. The gateway re-reads the table, so an
//! edit takes effect on the next message without a restart.

use serde_json::{Map, Value, json};

/// Actor id the owner has while no `[[person]]` says otherwise.
pub const IMPLICIT_OWNER: &str = "per_owner";

/// One `[[person]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonSpec {
    pub actor_id: String,
    pub is_owner: bool,
    pub roles: Vec<String>,
    /// `(body_id, account_id)`.
    pub accounts: Vec<(String, String)>,
}

/// One change to the person table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeopleEdit {
    Create { actor_id: String, roles: Vec<String>, account: Option<(String, String)> },
    Delete(String),
    SetOwner(String),
    /// Creates the person when it does not exist yet.
    AddAccount { actor_id: String, account: (String, String) },
    RemoveAccount { actor_id: String, account: (String, String) },
    SetRoles { actor_id: String, roles: Vec<String> },
}

fn valid_actor(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 || id.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("actor id must be one word (no spaces)".to_owned());
    }
    Ok(())
}

fn valid_roles(roles: &[String]) -> Result<(), String> {
    for r in roles {
        if r.is_empty() || !r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
            return Err(format!("role '{r}': use letters, digits, '_' or '-' only"));
        }
    }
    Ok(())
}

/// Splits `a, b` / `a b` into role names.
#[must_use]
pub fn parse_roles(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn owner_exists(persons: &[PersonSpec]) -> bool {
    persons.iter().any(|p| p.is_owner)
}

fn account_holder<'a>(persons: &'a [PersonSpec], acct: &(String, String)) -> Option<&'a str> {
    persons
        .iter()
        .find(|p| p.accounts.contains(acct))
        .map(|p| p.actor_id.as_str())
}

/// Applies `edit` to `persons`, enforcing a single owner and one holder per account.
///
/// # Errors
/// A message naming what is wrong.
pub fn apply(persons: &[PersonSpec], edit: &PeopleEdit) -> Result<Vec<PersonSpec>, String> {
    let mut out = persons.to_vec();
    match edit {
        PeopleEdit::Create { actor_id, roles, account } => {
            valid_actor(actor_id)?;
            valid_roles(roles)?;
            if out.iter().any(|p| &p.actor_id == actor_id) {
                return Err(format!("'{actor_id}' already exists"));
            }
            if let Some(a) = account
                && let Some(holder) = account_holder(&out, a)
            {
                return Err(format!("{} {} already belongs to '{holder}'", a.0, a.1));
            }
            out.push(PersonSpec {
                actor_id: actor_id.clone(),
                is_owner: actor_id == IMPLICIT_OWNER && !owner_exists(&out),
                roles: roles.clone(),
                accounts: account.iter().cloned().collect(),
            });
        }
        PeopleEdit::Delete(id) => {
            let Some(pos) = out.iter().position(|p| &p.actor_id == id) else {
                return Err(format!("no person '{id}'"));
            };
            if out[pos].is_owner {
                return Err("the owner cannot be deleted; make another person the owner first".to_owned());
            }
            out.remove(pos);
        }
        PeopleEdit::SetOwner(id) => {
            valid_actor(id)?;
            if !out.iter().any(|p| &p.actor_id == id) {
                out.push(PersonSpec {
                    actor_id: id.clone(),
                    is_owner: false,
                    roles: Vec::new(),
                    accounts: Vec::new(),
                });
            }
            for p in &mut out {
                p.is_owner = &p.actor_id == id;
            }
        }
        PeopleEdit::AddAccount { actor_id, account } => {
            valid_actor(actor_id)?;
            if account.0.is_empty() || account.1.is_empty() {
                return Err("enter a body id and an account id".to_owned());
            }
            if let Some(holder) = account_holder(&out, account)
                && holder != actor_id
            {
                return Err(format!("{} {} already belongs to '{holder}'", account.0, account.1));
            }
            if !out.iter().any(|p| &p.actor_id == actor_id) {
                let owner = actor_id == IMPLICIT_OWNER && !owner_exists(&out);
                out.push(PersonSpec {
                    actor_id: actor_id.clone(),
                    is_owner: owner,
                    roles: Vec::new(),
                    accounts: Vec::new(),
                });
            }
            if let Some(p) = out.iter_mut().find(|p| &p.actor_id == actor_id)
                && !p.accounts.contains(account)
            {
                p.accounts.push(account.clone());
            }
        }
        PeopleEdit::RemoveAccount { actor_id, account } => {
            let Some(p) = out.iter_mut().find(|p| &p.actor_id == actor_id) else {
                return Err(format!("no person '{actor_id}'"));
            };
            p.accounts.retain(|a| a != account);
        }
        PeopleEdit::SetRoles { actor_id, roles } => {
            valid_roles(roles)?;
            if let Some(p) = out.iter_mut().find(|p| &p.actor_id == actor_id) {
                p.roles = roles.clone();
            } else {
                valid_actor(actor_id)?;
                let owner = actor_id == IMPLICIT_OWNER && !owner_exists(&out);
                out.push(PersonSpec {
                    actor_id: actor_id.clone(),
                    is_owner: owner,
                    roles: roles.clone(),
                    accounts: Vec::new(),
                });
            }
        }
    }
    Ok(out)
}

/// The `config.apply` patch that replaces the whole `[[person]]` array.
#[must_use]
pub fn patch(persons: &[PersonSpec]) -> Value {
    if persons.is_empty() {
        return json!({ "person": null });
    }
    let arr: Vec<Value> = persons
        .iter()
        .map(|p| {
            let mut o = Map::new();
            o.insert("actor_id".to_owned(), json!(p.actor_id));
            if p.is_owner {
                o.insert("owner".to_owned(), json!(true));
            }
            if !p.accounts.is_empty() {
                o.insert(
                    "accounts".to_owned(),
                    json!(p
                        .accounts
                        .iter()
                        .map(|(b, a)| json!({ "body_id": b, "account_id": a }))
                        .collect::<Vec<_>>()),
                );
            }
            if !p.roles.is_empty() {
                o.insert("roles".to_owned(), json!(p.roles));
            }
            Value::Object(o)
        })
        .collect();
    json!({ "person": arr })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(b: &str, a: &str) -> (String, String) {
        (b.to_owned(), a.to_owned())
    }

    #[test]
    fn owner_account_is_added_to_an_implicit_owner() {
        let wa = acct("body-whatsapp", "wa:user:6285111242960@s.whatsapp.net");
        let out = apply(
            &[],
            &PeopleEdit::AddAccount { actor_id: IMPLICIT_OWNER.to_owned(), account: wa.clone() },
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].is_owner && out[0].accounts == vec![wa]);
        let p = patch(&out);
        assert_eq!(p["person"][0]["owner"], json!(true));
        assert_eq!(p["person"][0]["accounts"][0]["body_id"], json!("body-whatsapp"));
    }

    #[test]
    fn promoting_a_guest_creates_a_person_with_roles_and_the_account() {
        let out = apply(
            &[],
            &PeopleEdit::Create {
                actor_id: "per_ibu".to_owned(),
                roles: vec!["family".to_owned()],
                account: Some(acct("body-whatsapp", "wa:user:1@s.whatsapp.net")),
            },
        )
        .unwrap();
        assert_eq!(patch(&out)["person"][0]["roles"], json!(["family"]));
        assert!(!out[0].is_owner);
    }

    #[test]
    fn single_owner_unique_accounts_and_owner_delete_are_enforced() {
        let a = apply(&[], &PeopleEdit::SetOwner("per_a".to_owned())).unwrap();
        let b = apply(&a, &PeopleEdit::SetOwner("per_b".to_owned())).unwrap();
        assert_eq!(b.iter().filter(|p| p.is_owner).count(), 1);
        assert!(b.iter().find(|p| p.actor_id == "per_b").unwrap().is_owner);
        assert!(apply(&b, &PeopleEdit::Delete("per_b".to_owned())).is_err());
        assert!(apply(&b, &PeopleEdit::Delete("per_a".to_owned())).is_ok());
        let x = acct("tg", "1");
        let c = apply(&b, &PeopleEdit::AddAccount { actor_id: "per_a".to_owned(), account: x.clone() }).unwrap();
        assert!(apply(&c, &PeopleEdit::AddAccount { actor_id: "per_b".to_owned(), account: x }).is_err());
        assert!(apply(&c, &PeopleEdit::SetRoles { actor_id: "per_a".to_owned(), roles: vec!["bad role".to_owned()] }).is_err());
        assert_eq!(patch(&[]), json!({ "person": null }));
    }
}
