//! Governance and observability panels: `/approvals`, `/people`, `/policy`, `/activity`,
//! `/events` and `/status`. Each is a full-screen overlay (like `/log`) over data fetched with
//! the gateway's governance admin methods; the TUI keeps no authority of its own. State and key
//! handling live in the submodules, drawing in [`draw`], and the async glue (requests,
//! responses, polling) in [`GovState::due_requests`] and the runtime.

pub mod activity;
pub mod approvals;
pub mod events;
pub mod people;
pub mod policy;
pub mod status;

use std::collections::BTreeSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::render::{render_scrollbar, scroll_window};
use crate::state::AppAction;
use crate::theme::ThemePalette;
use crate::width::{chrome_label, fit_prefix, sanitize};

/// Rows used by the overlay besides the list (header and footer).
pub const GOV_CHROME_ROWS: u16 = 2;

/// Which panel is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Approvals,
    People,
    Policy,
    Activity,
    Events,
    Status,
}

impl Panel {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Approvals => "Approvals",
            Self::People => "People & links",
            Self::Policy => "Policy (read-only)",
            Self::Activity => "Adapter activity (read-only)",
            Self::Events => "Events",
            Self::Status => "Status",
        }
    }
}

/// What a response belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GovTag {
    Approvals,
    People,
    PeopleRecent,
    PeopleApply,
    Policy,
    Activity,
    Events,
    EventsMore,
    Explain,
    Trace,
    Status,
    LinkInitiate,
    LinkVerify,
    LinkUnlink,
    LabStart,
    LabJob,
}

/// One admin call the runtime performs for the TUI (carried by `AppAction::Gov`). `params` is
/// JSON text so the action stays `Eq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovRequest {
    pub tag: GovTag,
    pub method: String,
    pub params: String,
}

impl GovRequest {
    #[must_use]
    pub fn new(tag: GovTag, method: &str, params: &Value) -> Self {
        Self { tag, method: method.to_owned(), params: params.to_string() }
    }

    /// The parsed params.
    #[must_use]
    pub fn params_value(&self) -> Value {
        serde_json::from_str(&self.params).unwrap_or(Value::Null)
    }
}

/// Result of a [`GovRequest`].
#[derive(Debug, Clone)]
pub struct GovResponse {
    pub tag: GovTag,
    pub result: Result<Value, String>,
}

/// Row tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Dim,
    Good,
    Warn,
    Bad,
    Head,
}

/// One display row.
#[derive(Debug, Clone)]
pub struct Row {
    pub text: String,
    pub tone: Tone,
}

impl Row {
    #[must_use]
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self { text: text.into(), tone }
    }
}

/// Cursor and scroll position of a list.
#[derive(Debug, Clone, Copy, Default)]
pub struct Nav {
    pub selected: usize,
    pub offset: usize,
}

impl Nav {
    /// Moves the cursor by `delta` within `total` rows and keeps it on screen.
    pub fn step(&mut self, delta: isize, total: usize, visible: usize) {
        if total == 0 {
            self.selected = 0;
            self.offset = 0;
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(total - 1);
        self.offset = scroll_window(self.selected, self.offset, visible.max(1), total);
    }

    pub fn clamp(&mut self, total: usize) {
        self.selected = self.selected.min(total.saturating_sub(1));
    }

    /// Common navigation keys; returns whether `key` was one.
    pub fn handle_key(&mut self, key: KeyEvent, total: usize, visible: usize) -> bool {
        let page = visible.saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Up => self.step(-1, total, visible),
            KeyCode::Down => self.step(1, total, visible),
            KeyCode::PageUp => self.step(-page, total, visible),
            KeyCode::PageDown => self.step(page, total, visible),
            KeyCode::Home => self.step(isize::MIN / 2, total, visible),
            KeyCode::End => self.step(isize::MAX / 2, total, visible),
            _ => return false,
        }
        true
    }
}

/// A text box over the list (filter value, link input, trace id).
#[derive(Debug, Clone)]
pub struct Prompt {
    pub label: String,
    pub text: String,
    pub purpose: PromptFor,
}

/// What a finished prompt does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptFor {
    EventsPlace,
    EventsSource,
    EventsKind,
    EventsRange,
    EventsTrace,
    ActivitySearch,
    LinkInitiate { primary: Option<(String, String)> },
    LinkVerify,
    PersonCreate,
    PersonAddAccount { actor: String },
    PersonRoles { actor: String },
    PersonFromRecent { body: String, account: String },
}

/// A read-only text overlay (explain output, rule source, link instructions, job result).
#[derive(Debug, Clone)]
pub struct Detail {
    pub title: String,
    pub lines: Vec<String>,
    pub offset: usize,
}

/// Everything a panel needs to draw.
pub struct ViewModel {
    pub title: String,
    pub header: String,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub offset: usize,
    pub footer: String,
    /// Empty-list message.
    pub empty: String,
}

/// State of all governance panels.
#[derive(Debug, Clone, Default)]
pub struct GovState {
    pub panel: Option<Panel>,
    /// Gateway clock in ms (0 = use the system clock).
    pub now_ms: u64,
    /// List rows that fit on screen (kept in sync with the terminal size).
    pub view_rows: usize,
    pub approvals: approvals::ApprovalsView,
    pub people: people::PeopleView,
    pub policy: policy::PolicyView,
    pub activity: activity::ActivityView,
    pub events: events::EventsView,
    pub status: status::StatusView,
    /// Pending approvals (status-bar badge), kept fresh by polling even while no panel is open.
    pub pending_count: usize,
    pub detail: Option<Detail>,
    pub prompt: Option<Prompt>,
    /// Lab mode (replay/export only offered then); set by the state before every key.
    pub lab_mode: bool,
    inflight: BTreeSet<GovTag>,
    tick: u64,
}

/// Fallback clock when no gateway time is known.
#[must_use]
pub fn system_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `5s`, `3m`, `2h`, `1d`.
#[must_use]
pub fn age_label(now_ms: u64, ts_ms: u64) -> String {
    let s = now_ms.saturating_sub(ts_ms) / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// `HH:MM:SS` (UTC).
#[must_use]
pub fn clock_label(ts_ms: u64) -> String {
    let s = ts_ms / 1000 % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

pub(crate) fn jstr(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

pub(crate) fn ju64(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

impl GovState {
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    #[must_use]
    pub fn now(&self) -> u64 {
        if self.now_ms == 0 { system_now_ms() } else { self.now_ms }
    }

    fn visible(&self) -> usize {
        self.view_rows.max(1)
    }

    /// Opens `panel` and returns the requests that fill it.
    pub fn open(&mut self, panel: Panel) -> Vec<AppAction> {
        self.panel = Some(panel);
        self.detail = None;
        self.prompt = None;
        match panel {
            Panel::Approvals => self.approvals.confirm = None,
            Panel::People => {
                self.people.confirm = None;
                self.people.confirm_edit = None;
            }
            _ => {}
        }
        let now = self.now();
        let reqs = match panel {
            Panel::Approvals => vec![approvals::request()],
            Panel::People => vec![people::request(), people::recent_request()],
            Panel::Policy => vec![policy::request()],
            Panel::Activity => vec![activity::request()],
            Panel::Events => vec![self.events.query_request(now, false)],
            Panel::Status => vec![status::request()],
        };
        self.mark_inflight(reqs)
    }

    fn mark_inflight(&mut self, reqs: Vec<GovRequest>) -> Vec<AppAction> {
        reqs.into_iter()
            .map(|r| {
                self.inflight.insert(r.tag);
                AppAction::Gov(r)
            })
            .collect()
    }

    pub fn close(&mut self) {
        self.panel = None;
        self.detail = None;
        self.prompt = None;
    }

    /// Requests due on this 80 ms tick: the badge poll (about every 2 s), the open panel's
    /// refresh and running Lab jobs. A tag with a request still in flight is skipped.
    pub fn due_requests(&mut self) -> Vec<GovRequest> {
        self.tick += 1;
        let mut out = Vec::new();
        if self.tick.is_multiple_of(25) {
            out.push(approvals::request());
            match self.panel {
                Some(Panel::People) if self.prompt.is_none() => {
                    out.push(people::request());
                    out.push(people::recent_request());
                }
                Some(Panel::Activity) => out.push(activity::request()),
                Some(Panel::Status) => out.push(status::request()),
                _ => {}
            }
        }
        if self.tick.is_multiple_of(4) && self.events.job.as_ref().is_some_and(|j| j.running()) {
            out.push(self.events.job_request());
        }
        out.retain(|r| self.inflight.insert(r.tag));
        out
    }

    /// Applies one response.
    pub fn apply(&mut self, resp: GovResponse) -> Vec<AppAction> {
        self.inflight.remove(&resp.tag);
        let now = self.now();
        let mut follow_up = Vec::new();
        match (resp.tag, resp.result) {
            (GovTag::Approvals, Ok(v)) => {
                self.approvals.set(&v);
                self.pending_count = self.approvals.items.len();
            }
            (GovTag::People, Ok(v)) => self.people.set(&v),
            (GovTag::PeopleRecent, Ok(v)) => self.people.set_recent(&v),
            (GovTag::PeopleApply, Ok(_)) => {
                follow_up.push(AppAction::Gov(people::request()));
                follow_up.push(AppAction::Gov(people::recent_request()));
                self.inflight.insert(GovTag::People);
                self.inflight.insert(GovTag::PeopleRecent);
            }
            (GovTag::Policy, Ok(v)) => self.policy.set(&v),
            (GovTag::Activity, Ok(v)) => self.activity.set(&v),
            (GovTag::Events | GovTag::EventsMore, Ok(v)) => {
                self.events.set(&v, resp.tag == GovTag::EventsMore);
            }
            (GovTag::Trace, Ok(v)) => self.events.set_trace(&v),
            (GovTag::Status, Ok(v)) => self.status.set(&v),
            (GovTag::Explain, Ok(v)) => {
                let lines: Vec<String> = v
                    .get("lines")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                self.detail = Some(Detail { title: "Explain decision".to_owned(), lines, offset: 0 });
            }
            (GovTag::LinkInitiate, Ok(v)) => {
                self.detail = Some(people::initiate_instructions(&v));
            }
            (GovTag::LinkVerify | GovTag::LinkUnlink, Ok(v)) => {
                self.detail = Some(Detail {
                    title: "Link".to_owned(),
                    lines: vec![people::link_result_line(&v)],
                    offset: 0,
                });
                follow_up.push(AppAction::Gov(people::request()));
                self.inflight.insert(GovTag::People);
            }
            (GovTag::LabStart, Ok(v)) => {
                self.events.job_started(&v);
                follow_up.push(AppAction::Gov(self.events.job_request()));
                self.inflight.insert(GovTag::LabJob);
            }
            (GovTag::LabJob, Ok(v)) => {
                if let Some(d) = self.events.job_update(&v) {
                    self.detail = Some(d);
                }
            }
            (tag, Err(e)) => {
                let msg = format!("{}: {e}", tag_label(tag));
                match tag {
                    GovTag::Approvals | GovTag::Status | GovTag::Activity | GovTag::People | GovTag::PeopleRecent
                        if self.panel.is_none() =>
                    {
                        // Background poll while no panel is open: stay quiet.
                    }
                    GovTag::LabJob => {
                        self.events.job_failed(&e);
                    }
                    _ => {
                        self.detail = Some(Detail { title: "Error".to_owned(), lines: vec![msg], offset: 0 });
                    }
                }
            }
        }
        let _ = now;
        follow_up
    }

    /// Handles a key while a panel is open.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Vec::new();
        }
        // Prompt (a text box) owns the keyboard.
        if let Some(p) = &mut self.prompt {
            match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    let p = self.prompt.take().expect("prompt");
                    return self.finish_prompt(p);
                }
                KeyCode::Backspace => {
                    p.text.pop();
                    if p.purpose == PromptFor::ActivitySearch {
                        self.activity.search = p.text.clone();
                    }
                }
                KeyCode::Char(c) => {
                    p.text.push(c);
                    if p.purpose == PromptFor::ActivitySearch {
                        self.activity.search = p.text.clone();
                    }
                }
                _ => {}
            }
            return Vec::new();
        }
        // A detail overlay: scroll or close.
        if let Some(d) = &mut self.detail {
            let page = self.view_rows.max(2) - 1;
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.detail = None,
                KeyCode::Up => d.offset = d.offset.saturating_sub(1),
                KeyCode::Down => d.offset = (d.offset + 1).min(d.lines.len().saturating_sub(1)),
                KeyCode::PageUp => d.offset = d.offset.saturating_sub(page),
                KeyCode::PageDown => d.offset = (d.offset + page).min(d.lines.len().saturating_sub(1)),
                _ => {}
            }
            return Vec::new();
        }
        let Some(panel) = self.panel else {
            return Vec::new();
        };
        let visible = self.visible();
        let now = self.now();
        let acts = match panel {
            Panel::Approvals => self.approvals.handle_key(key, visible, now),
            Panel::People => self.people.handle_key(key, visible, &mut self.prompt, &mut self.detail),
            Panel::Policy => self.policy.handle_key(key, visible, &mut self.detail),
            Panel::Activity => self.activity.handle_key(key, visible, &mut self.prompt),
            Panel::Events => self.events.handle_key(key, visible, now, self.lab_mode, &mut self.prompt),
            Panel::Status => self.status.handle_key(key, visible),
        };
        let mut acts = acts;
        if matches!(key.code, KeyCode::Esc) && acts.is_empty() && self.prompt.is_none() && self.detail.is_none() {
            let consumed = match panel {
                Panel::Approvals => self.approvals.cancel_confirm(),
                Panel::People => self.people.cancel_confirm(),
                Panel::Events => {
                    let left = self.events.leave_trace();
                    if left {
                        acts.push(AppAction::Gov(self.events.query_request(now, false)));
                    }
                    left
                }
                _ => false,
            };
            if !consumed {
                self.close();
            }
        }
        let mut out = Vec::new();
        for a in acts {
            if let AppAction::Gov(r) = &a {
                self.inflight.insert(r.tag);
            }
            out.push(a);
        }
        out
    }

    fn finish_prompt(&mut self, p: Prompt) -> Vec<AppAction> {
        let now = self.now();
        let text = p.text.trim().to_owned();
        let reqs: Vec<AppAction> = match p.purpose {
            PromptFor::ActivitySearch => Vec::new(),
            PromptFor::EventsPlace | PromptFor::EventsSource | PromptFor::EventsKind | PromptFor::EventsRange => {
                match self.events.set_filter(&p.purpose, &text) {
                    Ok(()) => vec![AppAction::Gov(self.events.query_request(now, false))],
                    Err(e) => {
                        self.detail = Some(Detail { title: "Filter".to_owned(), lines: vec![e], offset: 0 });
                        Vec::new()
                    }
                }
            }
            PromptFor::EventsTrace => {
                if text.is_empty() {
                    Vec::new()
                } else {
                    self.events.trace_label = Some(text.clone());
                    vec![AppAction::Gov(events::trace_request(&text))]
                }
            }
            PromptFor::LinkInitiate { primary } => match people::initiate_request(primary, &text) {
                Ok(r) => vec![AppAction::Gov(r)],
                Err(e) => {
                    self.detail = Some(Detail { title: "Link".to_owned(), lines: vec![e], offset: 0 });
                    Vec::new()
                }
            },
            p @ (PromptFor::PersonCreate
            | PromptFor::PersonAddAccount { .. }
            | PromptFor::PersonRoles { .. }
            | PromptFor::PersonFromRecent { .. }) => match self.people.prompt_request(&p, &text) {
                Ok(r) => vec![AppAction::Gov(r)],
                Err(e) => {
                    self.detail = Some(Detail { title: "People".to_owned(), lines: vec![e], offset: 0 });
                    Vec::new()
                }
            },
            PromptFor::LinkVerify => match people::verify_request(&text) {
                Ok(r) => vec![AppAction::Gov(r)],
                Err(e) => {
                    self.detail = Some(Detail { title: "Link".to_owned(), lines: vec![e], offset: 0 });
                    Vec::new()
                }
            },
        };
        for a in &reqs {
            if let AppAction::Gov(r) = a {
                self.inflight.insert(r.tag);
            }
        }
        reqs
    }

    /// Mouse wheel while a panel is open.
    pub fn scroll_by(&mut self, delta: isize) {
        let visible = self.visible();
        if let Some(d) = &mut self.detail {
            d.offset = d.offset.saturating_add_signed(delta).min(d.lines.len().saturating_sub(1));
            return;
        }
        let total = self.view_model(self.now()).rows.len();
        match self.panel {
            Some(Panel::Approvals) => self.approvals.nav.step(delta, total, visible),
            Some(Panel::People) => self.people.nav.step(delta, total, visible),
            Some(Panel::Policy) => self.policy.nav.step(delta, total, visible),
            Some(Panel::Activity) => self.activity.nav.step(delta, total, visible),
            Some(Panel::Events) => self.events.nav.step(delta, total, visible),
            Some(Panel::Status) => self.status.nav.step(delta, total, visible),
            _ => {}
        }
    }

    /// Display model of the open panel.
    #[must_use]
    pub fn view_model(&self, now: u64) -> ViewModel {
        match self.panel {
            Some(Panel::Approvals) => self.approvals.view(now),
            Some(Panel::People) => self.people.view(),
            Some(Panel::Policy) => self.policy.view(),
            Some(Panel::Activity) => self.activity.view(now),
            Some(Panel::Events) => self.events.view(self.lab_mode),
            Some(Panel::Status) => self.status.view(),
            None => ViewModel {
                title: String::new(),
                header: String::new(),
                rows: Vec::new(),
                selected: None,
                offset: 0,
                footer: String::new(),
                empty: String::new(),
            },
        }
    }
}

fn tag_label(tag: GovTag) -> &'static str {
    match tag {
        GovTag::Approvals => "approvals",
        GovTag::People | GovTag::PeopleRecent | GovTag::PeopleApply => "people",
        GovTag::Policy => "policy",
        GovTag::Activity => "activity",
        GovTag::Events | GovTag::EventsMore => "events",
        GovTag::Explain => "explain",
        GovTag::Trace => "trace",
        GovTag::Status => "status",
        GovTag::LinkInitiate | GovTag::LinkVerify | GovTag::LinkUnlink => "link",
        GovTag::LabStart | GovTag::LabJob => "lab",
    }
}

fn pad(s: &str, width: usize) -> String {
    let fitted = fit_prefix(s, width);
    let used = fitted.width();
    format!("{fitted}{}", " ".repeat(width.saturating_sub(used)))
}

/// Draws the open panel over the whole `area`.
pub fn draw(
    area: Rect,
    buf: &mut Buffer,
    gov: &GovState,
    palette: &ThemePalette,
    icons: crate::config::IconMode,
) {
    if area.width < 10 || area.height < GOV_CHROME_ROWS + 1 {
        return;
    }
    let bg = Style::default().fg(palette.fg).bg(palette.bg);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(bg);
            }
        }
    }
    let vm = gov.view_model(gov.now());
    let visible = (area.height - GOV_CHROME_ROWS) as usize;
    let w = area.width as usize;
    let list_w = w.saturating_sub(1);

    let header = format!(" {}  {}", vm.title, vm.header);
    buf.set_string(
        area.x,
        area.y,
        pad(&chrome_label(&header, icons), w),
        Style::default().fg(palette.accent).bg(palette.status_bg).add_modifier(Modifier::BOLD),
    );

    let footer_text = if let Some(p) = &gov.prompt {
        format!(" {}: {}▏  (Enter apply, Esc cancel)", p.label, p.text)
    } else if gov.detail.is_some() {
        " Up/Down PgUp/PgDn scroll  Esc close".to_owned()
    } else {
        vm.footer.clone()
    };

    if let Some(d) = &gov.detail {
        buf.set_string(
            area.x,
            area.y + 1,
            pad(&chrome_label(&format!(" {}", d.title), icons), w),
            Style::default().fg(palette.accent).bg(palette.bg).add_modifier(Modifier::BOLD),
        );
        for (i, line) in d.lines.iter().skip(d.offset).take(visible.saturating_sub(1)).enumerate() {
            buf.set_string(
                area.x,
                area.y + 2 + i as u16,
                pad(&chrome_label(&format!(" {line}"), icons), list_w),
                Style::default().fg(palette.fg).bg(palette.bg),
            );
        }
        let sb = Rect::new(area.x + area.width - 1, area.y + 2, 1, visible.saturating_sub(1) as u16);
        render_scrollbar(sb, buf, d.lines.len(), d.offset, visible.saturating_sub(1), palette);
    } else {
        let total = vm.rows.len();
        let offset = scroll_window(vm.selected.unwrap_or(0), vm.offset, visible, total);
        for (i, row) in vm.rows.iter().skip(offset).take(visible).enumerate() {
            let fg = match row.tone {
                Tone::Normal => palette.fg,
                Tone::Dim => palette.dim,
                Tone::Good => palette.status_green,
                Tone::Warn => palette.status_yellow,
                Tone::Bad => palette.status_red,
                Tone::Head => palette.accent,
            };
            let mut style = Style::default().fg(fg).bg(palette.bg);
            if row.tone == Tone::Head {
                style = style.add_modifier(Modifier::BOLD);
            }
            if vm.selected == Some(offset + i) {
                style = style.bg(palette.status_bg).add_modifier(Modifier::BOLD);
            }
            buf.set_string(
                area.x,
                area.y + 1 + i as u16,
                pad(&chrome_label(&row.text, icons), list_w),
                style,
            );
        }
        if total == 0 {
            buf.set_string(
                area.x + 1,
                area.y + 1,
                fit_prefix(&sanitize(&vm.empty, false), w.saturating_sub(2)),
                Style::default().fg(palette.dim).bg(palette.bg),
            );
        }
        let sb = Rect::new(area.x + area.width - 1, area.y + 1, 1, visible as u16);
        render_scrollbar(sb, buf, total, offset, visible, palette);
    }

    buf.set_string(
        area.x,
        area.y + area.height - 1,
        pad(&chrome_label(&footer_text, icons), w),
        Style::default().fg(palette.dim).bg(palette.status_bg),
    );
}

/// Performs `req` against the gateway in a background task and reports back as
/// [`crate::client::RuntimeEvent::Gov`].
pub fn spawn(
    client: &Option<std::sync::Arc<dyn crate::client::GatewayClient>>,
    tx: &tokio::sync::mpsc::UnboundedSender<crate::client::RuntimeEvent>,
    req: GovRequest,
) {
    use crate::client::RuntimeEvent;
    let tx = tx.clone();
    let Some(c) = client.clone() else {
        let _ = tx.send(RuntimeEvent::Gov(GovResponse {
            tag: req.tag,
            result: Err("no gateway connection".to_owned()),
        }));
        return;
    };
    tokio::spawn(async move {
        let result = c.gov_call(req.method.clone(), req.params_value()).await.map_err(|e| e.message);
        let _ = tx.send(RuntimeEvent::Gov(GovResponse { tag: req.tag, result }));
    });
}
