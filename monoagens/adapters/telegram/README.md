# Telegram adapter

An adapter of MonoAgens: it follows the rules every adapter follows ([spec.md](../../../docs/spec.md)
*Adapters*, *Waist 2: middleware ↔ adapters and plugins*). This file holds only what is specific
to Telegram.

- **What it does.** long polling with `allowed_updates` for every kind it reads; inline buttons and the command menu, edit, delete, reply, reactions, polls, stickers, media groups (one percept per group), locations, forum topics; a failed download is an error percept, never a crash loop; no history and no read marks on the Bot API.
- **Audiences.** A group reports its members now; a channel reports `unknown`.
