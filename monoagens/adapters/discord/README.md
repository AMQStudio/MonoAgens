# Discord adapter

An adapter of MonoAgens: it follows the rules every adapter follows ([spec.md](../../../docs/spec.md)
*Adapters*, *Waist 2: middleware ↔ adapters and plugins*). This file holds only what is specific
to Discord.

- **What it does.** discord.py (message content intent); servers, channels, threads, DMs; buttons and menus (acknowledged within Discord's 3 s inside the adapter); reactions and edits on uncached messages; `allowed_mentions` set.
- **Audiences.** A channel reports its members, or `unknown` when anyone who joins later can read
  its history.
- **Dependencies** (approved by the owner, installed only with this adapter): `discord.py`.
