# Design — `tui-settings-complete`

Audit: [../tui-foundation/audit.md](../tui-foundation/audit.md).

## Context (facts checked on 2026-11-23)
- RBAC types: `crates/valve/src/rbac.rs` (`RbacConfig { fallback_role, roles, actor_roles, places, permission_bases }`,
  `RoleDefinition { name, base: RoleTier, allow, deny, approval: Option<ApprovalMode>, label, context, settings }`,
  `RoleTier = guest|member|trusted|admin|owner`, `ApprovalMode = yolo|default|strict`).
- Config: `crates/gateway/src/config.rs` (`PluginConfig` L105, `EmbedConfig` L321). Settings code:
  `crates/tui/src/settings.rs`, `state.rs`, `render.rs`; slash menu at `render.rs:1052`.
- Existing settings tabs: Appearance, Soul & Brain, Model, Valve, RBAC, Providers, Plugins, Runtime (8).
- `scroll_window` and `render_scrollbar` exist from `tui-foundation` (used by `/log` only).

## D7. Settings covers everything
- Field kinds: `Toggle`, `Select { options }` (popup list with search, scrollbar), `Number { min, max }`,
  `Text`, `Secret` (masked, `ctrl+r` reveal while editing), `List` (add / edit / delete / reorder items),
  `Map` (key/value rows), `Path` (validated exists), `Action` (button). Every enum in config uses `Select`.
- **Providers tab**: list of catalog providers grouped "Configured" / "Available"; each row shows key status
  (set, env, keyless, missing). Enter -> provider page: API key (Secret), base URL override (Text),
  `[ Test connection ]` (lists models, shows count + latency), `[ Use this provider ]`. Reuses the
  `model-provider-catalog` screens.
- **Model tab**: `Provider` (Select over configured providers), `Model` (Select over discovered models with
  search, plus "enter model id manually"), temperature, output limits, budget, `output_limit_param`
  (Select: `max_tokens` | `max_completion_tokens`), extra headers (Map), usage paths (Map), per-purpose models.
- **Embeddings**: backend (Select), model path (Path), endpoint, dim, timeout.
- **RBAC tab** (creates `[rbac]` on first edit): fallback role (Select over roles); Roles list -> role page:
  base tier (Select), approval mode (Select incl. "inherit"), label, context, allow/deny (List with a picker
  over known capability ids from connected bodies' `body.discover` plus free text), settings (Map);
  `[ + New role ]`, `[ Delete role ]` (refused while referenced by fallback/actor_roles, with the reason).
  Actors: list of `actor_roles`, add actor (pick from actors seen in the event log or type an id), assign
  roles (multi-select). Places: rules per place id (allow/deny lists). Permission bases: per capability id a
  Select of tier or "owner-only". The picker for capability ids and the actors seen in the log are read through
  the admin socket (`status`/`config.get` plus an events query from `tui-conversation`'s `events.query`);
  the exact source of "known capability ids" and "actors seen": verify during implementation.
- **Plugins**: detail page shows state with reason, last 5 stderr lines, `[ Start ] [ Stop ] [ Restart ]`,
  `Enable on startup`, required secrets from the plugin manifest/args with a warning when missing (e.g. telegram
  `TELEGRAM_BOT_TOKEN`), pairing status, typed editors for `plugins/<id>.toml` values (bool -> Toggle,
  number -> Number, array -> List, string -> Text), `[ View log ]` (opens `/log` filtered to `plugin:<id>`).
  Catalog entries: `[ Install ]` runs a guided flow (command path detection, kind, enable) instead of a bare
  key; Remove asks to confirm; a missing binary shows a build hint.
- **Coverage test**: a test walks every key that `GatewayConfig`/`tui.toml`/`secrets.toml` can parse and
  asserts a settings row (or documented exclusion with a reason, e.g. `version`) exists for it. How to
  enumerate parseable keys (serde schema via `schemars` or a hand-kept list checked against `Deserialize`
  defaults): proposed - confirm in review.
- **Writes**: every edit builds a patch and calls `config.apply`; secrets go to `secrets.toml` through the same
  admin path (proposed - confirm in review: a `secrets.set` admin method that never echoes values back).

## D4. Slash menu and scrolling
- The menu is drawn **above** the composer (it currently renders below the hint line and is squeezed to the
  remaining rows), max 10 rows, with its own scroll offset that keeps the selection visible, a scrollbar when
  items exceed rows, wrap-around Up/Down, PageUp/PageDown, mouse wheel and click, Tab completes the selected
  name. It stays open while the composer text starts with `/` and contains no space; it closes only on Esc,
  Enter, or when that condition stops holding.
- Shared `scroll_window` and `render_scrollbar` (from foundation) are used by the slash menu, settings lists,
  plugin detail, model picker and log.
- Esc in settings: closes an open editor -> leaves plugin detail/sub-page -> clears search -> closes overlay.

## D10 (part). UX patterns taken from omp v19.1.0 (observed in tmux on 2026-11-23)
Match the patterns, not omp's code.
- **Slash menu**: opens above the composer; selected row marked `❯`; a right-edge scrollbar (`█` thumb on a
  `│` track); each command's description shows **live state**. Ours: `/settings` -> "8 tabs", `/log` -> "3 errors",
  `/plugins` -> "4/6 running", `/model` -> current model.
- **Settings list**: section headings are rows inside the right list (the left index only jumps to them);
  scrollbar always on the right; the focused setting's one-line description sits in a footer under the list.
- **Enum selector**: Enter on an enum replaces the right pane with: name, description, a live preview where it
  applies, the option list (`❯` on current, scrollbar) and a "Type to search" line. Enter picks, Esc returns to
  the list unchanged.

## G4-G7 (proposals - confirm in review unless stated)
- **G4 Help**: `?` or `F1` opens an overlay listing every key bound on the current screen (generated from the
  same action table as G5, so it cannot drift).
- **G5 Keybindings**: `<config_dir>/keybindings.toml` maps action ids to key strings; defaults live in code;
  unknown ids or key conflicts are reported in `/log` and ignored. Editable from a settings section (a `Map`
  editor over action ids). File location and format are proposed (omp uses `keybindings.yml`).
- **G6 Command palette**: `Ctrl+K` opens a fuzzy list over commands, settings rows (jump to row) and chats;
  Enter runs/jumps. Fuzzy matcher crate vs hand-rolled subsequence scoring: decide during implementation.
- **G7 First-run wizard**: steps = provider (catalog list) -> key + test -> model picker -> plugin selection;
  never asks for a raw endpoint unless the provider is `custom`.
- **C2 Valve toggles**: each toggle row shows a one-line description of what it switches and, where cheap to
  compute, a live effect preview (for example "quiet hours 23:00-07:00: next turn would be held"). Descriptions
  come from a static table in `crates/tui`; the preview content is verify during implementation.

## D3. In-TUI pairing
- A pairing page for adapters that pair (WhatsApp): renders the QR code (unicode half-blocks) or pair code that
  the plugin emits, with live status (waiting / scanned / paired / failed) until done; no subprocess takes over
  the terminal. How the plugin emits the code today (stdout of `monoagens pair`?) and whether a Waist-2 lifecycle
  event carries it: verify during implementation by reading `crates/cli` `pair` and the whatsapp plugin.
  For an attached TUI the owner process must run the pairing: proposed admin methods `plugins.pair.start` and
  `plugins.pair.status` (subscription) - confirm in review. No Waist-2 change.
- The old subprocess path stays as a fallback only if the plugin cannot emit machine-readable pairing output.
