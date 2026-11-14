# Web Search plugin

A plugin of MonoAgens: it follows the rules every plugin follows ([spec.md](../../docs/spec.md)
*Plugins*). This file holds only what is specific to Web Search.

- Tools `web_search` and `web_fetch`; permissions `web-search.search` and `web-search.fetch`,
  base `guest`.
- Providers in an ordered fallback chain set in `options`: SearXNG, Tavily, Brave, Exa,
  Serper, Google, and DDGS always last. Each normalizes its results to one shape and fails
  with a structured reason, never a crash. Port V1's `engine/tools/web_search.py` (ADR
  `engine/0026-search`).
- `web_fetch` runs in the MonoAgens process, so it refuses internal addresses (SSRF): resolve
  the host, check every address against the same blocked set as the sandbox, connect to
  the checked address only. Port V1's `fetch.py`.

**Dependencies** (approved by the owner, installed only with this plugin): `ddgs`.
