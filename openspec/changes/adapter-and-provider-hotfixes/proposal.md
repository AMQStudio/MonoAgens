# Proposal — `adapter-and-provider-hotfixes`

## Why
Found on 2026-11-24 with the owner's live setup (release build of a42c088; `~/.local/share/monoagens/logs/tui.log` and the event log read through the admin socket `events.query` / `events.explain`):

1. **WhatsApp never answers.** Inbound percepts arrive (`#456, #602, #606, #618, #622`, place `wa:dm:130799689031758@lid`) and every one is refused at the inbound door: `door_verdict door=inbound rule=valve.inbound.cedar reason_tag=refuse: unknown_audience` (`!known_actor && !room_present`). WhatsApp now addresses this DM by **LID** (`…@lid`). The owner's account was known as `whatsapp:6285111242960` (person `p_15b623ff`, seen in `/whoami` the day before). `plugins/whatsapp/src/inbound.rs` builds `actor = jid_to_actor(sender_jid)` and `place = jid_to_place(chat_jid)` from the LID and ignores `MessageSource.sender_alt` (wacore 0.7 `types/message.rs:104`), which carries the phone-number JID. The valve is right to refuse an unknown actor; the adapter translated wrongly. Even `/help` is refused because the inbound door runs first.
2. **The owner's API key was sent to ~20 other providers.** At startup model discovery ran for every catalog provider using the legacy `model_api_key` as fallback key (lookup order from `model-provider-catalog`: secrets → env → legacy key). OpenAI answered "Incorrect API key provided: oc_sk_fd…fEyz"; Anthropic, Google (key in the URL query), DeepSeek, Groq, Moonshot, xAI, Mistral, Together, Z.ai, DashScope, Cerebras, MiniMax, DeepInfra and Fireworks answered 401/400. Keyless local providers (ollama, lm-studio, llama-cpp, vllm) produced network-error warnings at every start.
3. **Model switching in the hosting TUI does not apply**: log `model/embed settings changed; provider rebuild hook not wired yet`.
4. **Telegram**: first connects time out (`connect failed: body RPC timed out after 5000ms`, three times) and `getUpdates` is retried every ~2.25 s while Telegram says `429 … retry after 14` (counting down), i.e. `retry_after` is ignored.
5. **Misleading supervisor log**: an intentional stop/restart logs `plugin 'X' stopped` twice and `ERROR … body 'body-X' disconnected/crashed`.

## What Changes
- WhatsApp adapter: stable account identity across LID / phone-number addressing (see spec).
- Provider keys are scoped: the legacy key only ever goes to the legacy endpoint; discovery runs only for configured providers, on demand.
- Provider rebuild hook wired in the TUI-hosted gateway.
- Telegram body answers the Waist-2 handshake before any network call and honours `retry_after`.
- Supervisor logs intentional stops as info, once.

## Impact
`plugins/whatsapp/src/{inbound.rs,body.rs,transport.rs}`, the telegram body (locate its crate), `crates/reason/src/secrets.rs`, `crates/reason/src/model/discovery.rs`, `crates/gateway/src/{provider.rs,owner.rs,supervisor.rs}`, `crates/tui/src/{runtime.rs,catalog_ui.rs,model_picker.rs}`. No core, valve or protocol change: the valve's refusal stays exactly as is.
