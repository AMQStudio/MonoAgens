# Design — `interactive-tui`

## Visual & Interaction Principles
- **Borderless calm aesthetic (OMP + Claude Code)**: Avoid heavy double-line ASCII frames around the full screen. Use thin horizontal rules (`───`) around the composer/status line, a single thin vertical separator (`│`) for the optional left sidebar and capability step blocks (`┌─ ... └─`), and a left accent bar (`▎`) for inline approval cards.
- **Chat-app message ergonomics**:
  - In local 1:1 chats (`Direct` and named local chats under `CHATS`), user messages render as dark-background right-aligned bubbles (max ~70% of chat viewport width, padded, no `You` label) and Mono's replies render cleanly left-aligned on the canvas (no `Mono` label).
  - In `/lab` simulated multi-actor groups, other simulated speakers render left-aligned with muted colored sender names while the active speaker renders right-aligned in a dark bubble.
  - Users can switch `Message Layout` between `chat` (default) and `stream` (full-width) in `/settings -> Appearance`.
- **Single-soul `CHATS` list (not conventional sessions)**:
  - `CHATS` only lists local terminal places (starting with `Direct` mapped to `cli:terminal`, plus user-created local chats via `/chat new` mapped to `cli:chat:<slug>`).
  - External WhatsApp/Telegram conversations are NOT mixed into the TUI `CHATS` sidebar; those live on their own apps while their plugin health appears in `PLUGINS`.
  - Every local chat belongs to the same persistent Mono soul and event log (`SegmentedStore`).
- **LLM Auto-Naming & Topic-Change Re-Titling**:
  - Each local chat stores `{ place_id, title, title_locked: bool, turns_since_retitle: u32, last_topic_summary: String }` persisted cleanly in `<data_dir>/tui_chats.json`.
  - On the **first user message** of an untitled/default local chat (when `auto_chat_title == true` and `!title_locked`), a non-blocking background task invokes the configured `ModelProvider` (`purpose: Classify`, short `<= 32` token prompt) to generate a concise 2–5 word title and updates the `CHATS` sidebar seamlessly.
  - On subsequent turns (evaluated when topic drift is detected via low lexical/embedding overlap against recent turns or every `retitle_check_interval` turns when `!title_locked`), a non-blocking background classification asks whether the conversation topic shifted significantly and, if so, updates the chat title to match the new topic.
  - Explicit `/chat rename <title>` sets `title_locked = true` so automatic re-titling never overrides a user-chosen name.
  - Fails silently/gracefully (keeping existing title) when offline, scripted, or rate-limited—never blocking chat turns!

## Sidebar `PLUGINS` Traffic-Light State Machine
Each plugin entry tracks live lifecycle state rendered with clear color + kind badge `(Adapter)` or `(Capabilities)`:
- **Green (`●`)**: `PluginUiState::Running` (connected & healthy)
- **Yellow (`◐`)**: `PluginUiState::Starting` or `PluginUiState::Stopping` (in-flight spawn/connect or graceful shutdown)
- **Red (`●`)**: `PluginUiState::Stopped` or `PluginUiState::Failed(String)` (disabled, stopped, or errored)

## Full 8-Tab `/settings` Architecture & Generic Plugin Drill-Down
Inspired by OMP's 2-pane Settings + Claude Code's search/config overlay:
- **Tabs**: `Appearance`, `Soul & Brain`, `Model`, `Valve`, `RBAC`, `Providers`, `Plugins`, `Runtime`.
- **Navigation**: `←`/`→` switches tabs, `Tab` jumps left section anchors, `↑`/`↓` or mouse wheel scrolls options, typing printable chars filters via live search, `Enter`/`Space` toggles bools, cycles enums, or opens inline text/number input (masked for secrets), `Esc` closes or returns from plugin drill-down.
- **Persistence**:
  - Core/Gateway/RBAC/Plugin enablement changes write via `toml_edit` to `monoagens.toml` (creating `monoagens.toml.bak` first).
  - Secret/API-key/`[env]` changes write to `secrets.toml` with `0600` Unix permissions (never displaying plaintext values after entry).
  - TUI-only appearance preferences (`theme`, `sidebar`, `message_layout`, `motion`, `shimmer`, `mouse_support`, `auto_chat_title`, `show_capability_steps`, `show_turn_duration`, `show_brain_trace`) persist in `<config_dir>/tui.toml` so `monoagens.toml` stays lean and free of UI bloat!
- **Plugins Tab Drill-Down**:
  - Level 1 lists all configured + available built-in plugins with 3-color status dots, `[Adapter]`/`[Capabilities]` badges, `Space` start/stop, `i` install, `d` remove, `Enter` configure.
  - Level 2 (`Plugins › <id>`) provides:
    - Action items: `Start / Stop Plugin`, `Pair Device` (suspends raw mode + mouse capture cleanly, runs `<bin> pair --db <state_dir>/session.db` interactively in TTY, restores TUI on exit), `Enable on Startup`, `Manage Env Secrets`.
    - Generic dynamic TOML editor loading `<config_dir>/plugins/<id>.toml` (triggering default creation or editing arbitrary bool/int/float/string/array keys so any third-party international plugin is configurable from the TUI without hardcoding!).

## Command-Gated `/lab` Mode
- No mode selector tabs exist in the header.
- `/lab` activates Lab simulation state: adds a subtle `lab` badge to the bottom status line, unlocks `/as <actor>`, `/group <name> <members...>`, `/dm <actor>`, `/brain`, and reveals the `SPEAK AS` section in the sidebar plus dim brain telemetry lines (`Silent`/`Go`, `p_go`, tier, `W_fast`).
- `/exit` is context-aware: while inside `/lab`, `/exit` (or `/lab exit`) exits Lab mode back to normal chat without quitting the harness; when already in normal chat mode, `/exit` quits MonoAgens gracefully.

## Full Mouse Support, Bracketed Paste & Select-to-Auto-Copy
- Enabled via `crossterm::event::EnableMouseCapture` and `EnableBracketedPaste` when active (both cleanly restored on exit/panic/suspend; `mouse_support` and `copy_on_select` default `true` in `tui.toml` and `/settings -> Appearance`).
- Layout pass records hit-test rectangles (`HitRegions`: sidebar chat rows, sidebar plugin rows, chat viewport + per-row rendered text lines, attachment card dismiss targets, approval `[ Approve ]` / `[ Deny ]` buttons, slash autocomplete rows, settings tab pills, settings section rows, settings option rows, plugin drill-down action rows).
- Supports wheel scrolling (`ScrollUp`/`ScrollDown`) in whichever pane the cursor hovers over, left-click (`MouseEventKind::Down(MouseButton::Left)`) on all interactive targets, and mouse drag-selection (`Down` -> `Drag` -> `Up`) inside the chat viewport with visible highlight + automatic clipboard copy on mouse release via OSC 52 (`\x1b]52;c;<base64>\x07`) plus optional system clipboard command fallback (`wl-copy`, `xclip`, `xsel`, `pbcopy`) and `"Copied to clipboard"` toast. `/copy` copies the latest assistant reply.

## Smart Attachment Cards, Long-Text Paste & Workspace-Gated File Import
- **Attachment strip above composer**: renders compact rounded numbered preview cards (`╭─ 📷 #1 ─╮` ... `╰─ 640x640 ─╯` for images with pure-Rust header dimension parsing, `╭─ 📄 #2 ─╮` ... `╰─ report.pdf · 42 KB ─╯` for PDFs, `╭─ 📝 #3 ─╮` ... `╰─ 42 lines · 2.1 KB ─╯` for long pasted text), paired with inline tokens (`📷 #1`, `📄 #2`, `📝 #3`) inside the composer.
- **Native Media/Text vs. Workspace-Gated Files**:
  - **Images** (`png`, `jpg`, `jpeg`, `webp`, `gif`, plus `Ctrl+V` clipboard images), **PDFs** (`pdf`), and **long pasted text** (`lines >= 8 || chars >= 600`) work natively regardless of whether `workspace` is running, becoming numbered `#N` preview cards.
  - **All other host files** (`.jar`, `.zip`, source/text/data files dropped/pasted or `/attach`ed) are **strictly gated on the `workspace` plugin being active (`Running`)**:
    - When `workspace` is **active**: the TUI copies the file into the soul's sandboxed workspace (`<workspace_root>/<actor_key>/inbox/<filename>`, resolving `root` from `<config_dir>/plugins/workspace.toml` or `<data_dir>/plugins/workspace/workspaces`) and rewrites the composer text to the workspace-relative path (`inbox/<filename>`)—never leaking raw host paths!
    - When `workspace` is **inactive**: the file is refused with a clear toast explaining that the `workspace` plugin must be active to import files as paths into Mono's workspace.
  - Short non-path pasted text inserts directly into the composer.
  - `Ctrl+V` checks the system clipboard for image data (`wl-paste --type image/png`, `xclip -selection clipboard -t image/png -o`, or macOS clipboard) and attaches it directly as a `📷 #N` card with dimensions; if no image is present, reads clipboard text and applies the exact same smart classifier.
- **Submission**: Attached images/PDFs are staged into `<data_dir>/tui_attachments/` with Waist-2 `AttachmentHandle`s (never exposing raw host paths in percept notes); pasted long-text attachments expand cleanly into the submitted turn payload while keeping the chat bubble compact (`📝 #1 · 42 lines`). Clicking a card's dismiss area, `/detach [id]`, or removing the `#N` token detaches the item before sending.

## Animations (`tachyonfx`) & Terminal Safety
- Uses `tachyonfx::EffectManager` ticked at ~30 FPS (only when active effects or thinking shimmer exist to keep idle CPU near 0%!).
- Effects: message fade-in (`160 ms`), status-line thinking shimmer + braille spinner, modal dissolve (`120 ms`), toast slide/fade (`2.5 s`).
- `MotionMode::{Full, Reduced, Off}` (automatically `Off` if `NO_COLOR` is set or user disables motion).
- Panic hook & `Drop` guard ALWAYS restore terminal raw mode, alternate screen, and disable mouse capture so the user's terminal is never left corrupted on crash or `Ctrl+C`.

## Testability (Headless Ratatui `TestBackend`)
All state transitions, keyboard & mouse hit-testing, theme rendering, `/lab` gating, 8-tab `/settings` search & TOML persistence, dynamic plugin TOML editing, 3-color plugin status transitions, and LLM auto-titling / topic-drift re-titling are unit- and integration-tested headlessly using `ratatui::backend::TestBackend` and `ScriptedModelProvider` without requiring a physical TTY.
