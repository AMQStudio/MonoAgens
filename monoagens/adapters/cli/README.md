# CLI adapter

An adapter of MonoAgens: it follows the rules every adapter follows ([spec.md](../../../docs/spec.md)
*Adapters*, *Waist 2: middleware ↔ adapters and plugins*). This file holds only what is specific
to CLI.

- **What it does.** Unix socket inside the main process, thin client; text, files by path, reply; `composing` and `progress` as status lines; questions as numbered choices.
