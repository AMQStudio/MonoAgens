# WhatsApp adapter

An adapter of MonoAgens: it follows the rules every adapter follows ([spec.md](../../../docs/spec.md)
*Adapters*, *Waist 2: middleware ↔ adapters and plugins*). This file holds only what is specific
to WhatsApp.

- **What it does.** neonize (a linked device); own or owner's number; full history import; files, stickers, polls (votes decrypted), reactions, edits; no buttons, so questions by poll; handles logout and disconnect.
- **One stable form per actor.** WhatsApp gives one account two forms (the LID and the
  phone-number JID); the adapter reports one, so one person never looks like two.
- **History.** On pairing, WhatsApp's history sync is imported whole (`imported: true`).
- **An account shared with a person.** As a linked device on the owner's own number, the adapter
  tells the soul's messages from the owner's by the native ids in the Log.
- **Audiences.** A group reports its members now (newcomers do not see the past); a Channel
  reports `unknown`.
- **Dependencies** (approved by the owner, installed only with this adapter): `neonize` with `segno` and its `protobuf` override.
