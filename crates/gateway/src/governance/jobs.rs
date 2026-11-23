//! Lab jobs: offline replay and learned-state export as cancellable background jobs.
//!
//! Both run on a snapshot of the event log (`read_events` clones it through the owner task)
//! and an in-memory store, never on the live store: nothing is appended and the live queue is
//! never fed. No model is called (replay uses the recorded decisions). Progress is polled with
//! `lab.job`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use monoagens_core::ReplayMode;
use serde_json::{Value, json};

use crate::owner::{AdminError, ERR_METHOD_NOT_FOUND, GatewayOwner};

use super::events::replay_events;
use super::export::{JobCtl, export_learning};

struct Job {
    kind: &'static str,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
    result: Mutex<Option<Result<Value, String>>>,
}

/// Registry of Lab jobs owned by one [`GatewayOwner`].
#[derive(Default)]
pub struct Jobs {
    next: AtomicU64,
    jobs: Mutex<BTreeMap<u64, Arc<Job>>>,
}

impl Jobs {
    fn add(&self, kind: &'static str) -> (u64, Arc<Job>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let job = Arc::new(Job {
            kind,
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            result: Mutex::new(None),
        });
        let mut map = self.jobs.lock().expect("jobs lock");
        // Keep the registry small: forget the oldest finished jobs.
        while map.len() >= 16 {
            let finished = map
                .iter()
                .find(|(_, j)| j.result.lock().expect("job result").is_some())
                .map(|(k, _)| *k);
            match finished {
                Some(k) => {
                    map.remove(&k);
                }
                None => break,
            }
        }
        map.insert(id, Arc::clone(&job));
        (id, job)
    }

    fn get(&self, id: u64) -> Option<Arc<Job>> {
        self.jobs.lock().expect("jobs lock").get(&id).cloned()
    }
}

fn job_json(id: u64, job: &Job) -> Value {
    let result = job.result.lock().expect("job result").clone();
    let (state, result_json, error) = match result {
        None => ("running", Value::Null, Value::Null),
        Some(Ok(v)) => ("done", v, Value::Null),
        Some(Err(e)) if e == "cancelled" => ("cancelled", Value::Null, Value::Null),
        Some(Err(e)) => ("failed", Value::Null, json!(e)),
    };
    json!({
        "job": id,
        "kind": job.kind,
        "state": state,
        "done": job.done.load(Ordering::Relaxed),
        "total": job.total.load(Ordering::Relaxed),
        "result": result_json,
        "error": error,
    })
}

fn spawn_job<F>(job: Arc<Job>, events: Vec<monoagens_core::EventRecord>, work: F)
where
    F: FnOnce(&[monoagens_core::EventRecord], &JobCtl<'_>) -> Result<Value, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let progress_job = Arc::clone(&job);
        let progress = move |done: u64, total: u64| {
            progress_job.done.store(done, Ordering::Relaxed);
            progress_job.total.store(total, Ordering::Relaxed);
        };
        let ctl = JobCtl { progress: &progress, cancel: &job.cancel };
        let res = work(&events, &ctl);
        if res.is_ok() {
            let total = job.total.load(Ordering::Relaxed);
            job.done.store(total, Ordering::Relaxed);
        }
        *job.result.lock().expect("job result") = Some(res);
    });
}

pub(crate) async fn lab_call(
    owner: &GatewayOwner,
    method: &str,
    params: &Value,
) -> Result<Value, AdminError> {
    let jobs = owner.jobs();
    match method {
        "lab.replay" => {
            let mode = match params.get("mode").and_then(Value::as_str) {
                Some("unpinned") => ReplayMode::Unpinned,
                _ => ReplayMode::Pinned,
            };
            let events = super::read_events(owner).await?;
            let (id, job) = jobs.add("replay");
            job.total.store(events.len() as u64, Ordering::Relaxed);
            spawn_job(Arc::clone(&job), events, move |ev, ctl| {
                let summary = replay_events(ev, mode, ctl)?;
                serde_json::to_value(&summary).map_err(|e| e.to_string())
            });
            Ok(json!({ "job": id }))
        }
        "lab.export_learning" => {
            let include_private = params
                .get("include_private")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let place = params.get("place").and_then(Value::as_str).map(str::to_owned);
            let out = params
                .get("out")
                .and_then(Value::as_str)
                .map_or_else(
                    || {
                        let ts = owner.handle().clock().now().as_millis();
                        owner.data_dir().join("exports").join(format!("learning-{ts}.md"))
                    },
                    std::path::PathBuf::from,
                );
            let events = super::read_events(owner).await?;
            let (id, job) = jobs.add("export_learning");
            job.total.store(events.len() as u64, Ordering::Relaxed);
            spawn_job(Arc::clone(&job), events, move |ev, ctl| {
                let exp = export_learning(ev, include_private, place.as_deref(), ctl)?;
                if let Some(parent) = out.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create export dir: {e}"))?;
                }
                std::fs::write(&out, exp.doc.as_bytes())
                    .map_err(|e| format!("failed to write export '{}': {e}", out.display()))?;
                let mut v = serde_json::to_value(&exp).map_err(|e| e.to_string())?;
                v["out"] = json!(out.display().to_string());
                Ok(v)
            });
            Ok(json!({ "job": id }))
        }
        "lab.job" | "lab.cancel" => {
            let id = params
                .get("job")
                .and_then(Value::as_u64)
                .ok_or_else(|| AdminError::invalid_params("missing integer param 'job'"))?;
            let job = jobs
                .get(id)
                .ok_or_else(|| AdminError::invalid_params(format!("no such job {id}")))?;
            if method == "lab.cancel" {
                job.cancel.store(true, Ordering::Relaxed);
            }
            Ok(job_json(id, &job))
        }
        other => Err(AdminError::new(
            ERR_METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        )),
    }
}
