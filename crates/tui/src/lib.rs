//! Interactive Ratatui + Crossterm + TachyonFX terminal UI for MonoAgens.

#![forbid(unsafe_code)]

pub mod attachments;
pub mod clipboard;
pub mod config;
pub mod effects;
pub mod markdown;
pub mod render;
pub mod runtime;
pub mod settings;
pub mod state;
pub mod theme;
pub mod titler;

pub use attachments::{
    FileAttachmentOutcome, MAX_INLINE_DOCUMENT_BYTES, MAX_TUI_PERCEPT_INLINE_BYTES, PasteItem,
    PendingAttachment, PendingAttachmentKind, build_percept_payload_and_attachments,
    classify_native_media_extension, classify_paste_input, format_byte_size,
    import_host_file_to_workspace, inspect_file_for_attachment, parse_image_dimensions,
    resolve_workspace_root, sanitize_actor_key,
};
pub use clipboard::{
    ClipboardContent, copy_to_clipboard, encode_base64, osc52_sequence, osc52_sequence_with_tmux,
    read_clipboard_image_or_text,
};
pub use config::{
    BuiltinPluginPreset, DynamicTomlDoc, DynamicTomlEntry, DynamicTomlValue, LocalChatEntry,
    LocalChatStore, MessageLayout, MotionMode, ShimmerMode, SidebarMode, StoredChatMessagePreview,
    TuiPreferences,
};
pub use effects::TuiEffects;
pub use render::{render_app, render_to_buffer};
pub use runtime::{TerminalGuard, TuiLaunchOptions, run_tui, run_tui_with_provider};
pub use settings::{
    InlineEditState, InlineEditTarget, PluginDetailState, PluginUiEntry, PluginUiState,
    SecretsUiSummary, SettingRow, SettingValueKind, SettingsModalState, SettingsTab,
    SetupWizardState, ValveRuleTogglesState, WizardStep, build_settings_rows,
};
pub use state::{
    AppAction, ChatSelection, ChatTimelineItem, ComposerState, HitRegions, LabSimulationState,
    PendingApprovalCard, SlashCommandItem, ToastMessage, TuiAppState,
};
pub use theme::{ThemePalette, ThemePreset, no_color_active};
pub use titler::{
    TitleTrigger, clean_generated_title, cosine_similarity, extract_topic_ngrams,
    generate_chat_title, ngram_jaccard, should_request_title, should_request_title_with_embeddings,
};
