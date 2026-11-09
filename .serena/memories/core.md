# MonoAgens core map

Persistent single-identity agent harness, pure Python asyncio. Invariants are normative in `AGENTS.md`; Claude role/workflow in `CLAUDE.md`. Read them first.

## Source map
- `monoagens/engine/` — sacred core; transport/UI agnostic. Never imports `monoagens.gateways` or transport SDKs.
  - `inbound.py` — gateway seam contracts (`Percept`, `Address`, `Action`, `Receipt`, `GatewayCore`).
  - `agent.py` / `agent_delivery.py` — `Agent.handle()` ingress, `Agent.dispatch()` single outbound door.
  - `turn/` — per-thread coordinator, prompt assembly, approval flow.
  - `memory/` — `MemoryStore` (only canonical write port), reads via `memories_readable` view, privacy L0–L5.
  - `access/` — roles/RBAC (`roles.yaml`), identity, approvals, consent relay.
  - `tools/`, `commands/` — core tools/commands exporting `TOOL`/`COMMAND = Handler(...)`.
  - `config/`, `soul.py` — config dataclasses (drive templates).
  - `io.py` — bounded executor; `await io.run(...)` for any sync work in async paths.
  - `i18n.py` — `t()`; chat strings in `monoagens/lang/{en,id}.yaml`.
- `monoagens/gateways/<name>/` — transport bodies (`BaseGateway`, `@register_gateway`).
- `monoagens/providers/` — LLM/speech wire adapters; core never branches on provider name.
- `monoagens/admin/` — CLI admin, `templates.py` (generates root `config.yaml`/`roles.yaml`/`soul.yaml`), `yaml_io.py` (comment-preserving writes).
- `migrations/` — linear Alembic chain (only path for schema changes).
- `plugins/<name>/` — `plugin.yaml` + `plugin.py`; `digest:` must be recomputed on change.
- `scripts/` — `selfcheck.py` (invariants, 800-line cap, i18n parity), `types.py` (pyright), `adr.py`.
- `docs/v0.1.0/adr/` — current ADRs; `docs/archive/` is superseded, do not cite as current.
- `docs/v0.1.0/contracts/gateway-plugin.md` — normative gateway/plugin/MCP contract.

## Further memories
- Language, deps, tooling versions: `mem:tech_stack`.
- Dev/test/db commands: `mem:suggested_commands`.
- Style, module caps, typing, extension patterns: `mem:conventions`.
- Done-gate commands and sync steps: `mem:task_completion`.
