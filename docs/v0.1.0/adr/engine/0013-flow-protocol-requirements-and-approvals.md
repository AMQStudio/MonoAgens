# ADR-0013: Flow Protocol, Requirements, and Generic Approvals — Plugins Declare Needs, the Core Runs Durable Flows, Gateways Render

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Approvals are detected by a regex over outgoing text, a reaction can bind any bot message to any pending approval, settings reads are parked as approvals, privileged roles can decide another person's consent, and plugin preconditions (login, file sharing, consent) have no home. This ADR gives every layer one protocol for "what I need before I can continue" and one durable runner for it.
- **Supersedes:** in part ADR-0006 §4 (approval routing of 4.5), ADR-0010 §4 (the `/approve #N` badge of 4.1), ADR-0004 §4 (approval routing of 4.2)
- **Related:** ADR-0011 (charter), ADR-0012 (receipts), ADR-0014 (plugins and gateways), ADR-0003 (durability)
- **Implementation:** draft

---

## 1. Context

Audit evidence (`.audit/01-ingress-gateways.md`, `.audit/04-access-commands-guard.md`,
`.audit/05-tools-plugins.md`):

- `monoagens/gateways/base.py:40` `APPROVAL_BADGE = re.compile(r"/approve\s*#?(\d+)")` turns
  any reply that mentions `/approve #5` into an approval prompt; WhatsApp then appends a
  reaction hint from the core catalog and registers the message as an approval.
- `engine/guard/policy.py:578` parks every `settings` call, including reads; the observed
  `/approve #8` failure (`path not found: roles`) was a replayed read.
- `commands/approve.py:45` lets privileged roles decide privacy and disclosure rows on the
  subject's behalf; authority is derived from `tools: ["*"]`.
- Natural-language confirmation is matched from English keyword tables
  (`engine/character/lexicon.py:183`).
- The Google plugin needs login, access checks, sharing with a service account, and consent in
  groups; none of these steps can be expressed today.

## 2. Decision drivers

- A mechanism is data, never text parsed back out of chat.
- The core stays generic; plugin-specific rules are taught by plugins.
- Gateways choose the gesture; the core validates the decision.
- Flows survive restarts, respect the lease, and never repeat a side effect.
- The subject's veto cannot be bypassed by any role.

## 3. Options considered

1. **Fix the regex and keep approvals special.** Rejected: every plugin would re-invent waiting
   for login, sharing, or consent.
2. **External workflow engine.** Rejected: second system of record; violates the topology.
3. **Typed outcome protocol plus a minimal durable flow runner in PostgreSQL (chosen).**

## 4. Decision

### 4.1 Outcome protocol

Every plugin step and every core capability that may wait returns one of:

```
Done(result)
Refused(reason)
Needs(requirement, resume)
```

`resume` names the step to call when the requirement is satisfied, plus the state it needs.

### 4.2 Requirement kinds

The core knows only these kinds; plugins fill in the specifics:

| Kind | Meaning | Satisfied when |
|---|---|---|
| `Approval(deciders, action, expires)` | consent of specific people | a valid decider decides |
| `Connect(account_kind)` | the person links an external account | the plugin reports the account connected |
| `ExternalAction(instruction, verify)` | the person does something outside, such as sharing a file with the service account as editor | the plugin's `verify` step returns `Done` |
| `OperatorConfig(keys)` | a secret or setting is missing | the keys exist; until then the capability is not offered to the model |

### 4.3 Flow runner

- Build on the existing durable job engine (ADR-0005) wherever it already provides claiming,
  fencing, and retries; add only what it lacks. This is checked against the code before
  implementation; a new table is the fallback, not the default.
- The flow record holds: id, owner (core or plugin name), current step, state (JSON), awaited
  requirement (JSON), deciders, expiry, status, lease generation. The existing `approvals`
  table is migrated into it.
- One worker claims runnable flows with `FOR UPDATE SKIP LOCKED`, checks the lease fence, and
  calls the owner's step. Side effects reuse the outbox and action-id idempotency of ADR-0003.
- No DAGs, no branching language: a flow is ordinary code returning outcomes.
- Every state change writes a `system_receipt` message with the acting person (ADR-0012).

### 4.4 Events and decisions between core and gateway

- Core to gateway: `RequirementRaised` (with the requirement kind and data), `FlowFinished`,
  `AccessDenied` (person, reason). The gateway never renders `AccessDenied` in a shared room; in
  a direct chat it renders at most one notice per person per `access_notice_hours`.
- Gateway to core: `ApprovalDecision(flow, decided_by, approve)`,
  `ExternalActionDone(flow, by)`, `Confirmation(flow, by, choice)`.
- A gateway renders each kind natively (buttons, reactions, typed commands, a "done, check
  again" control) with its own strings; `monoagens.sdk` ships a plain-text fallback renderer.
- The core validates every decision: the decider is listed, RBAC allows it, the subject veto
  holds (for privacy and disclosure, only the subject decides), the flow is still pending.
- Reactions bind only to messages the gateway sent as the rendering of that requirement.

### 4.5 Approval policy corrections

- Approval need is derived from the handler's `approval_class` and action, not the tool name;
  `settings` reads never park.
- Authority to decide comes from the `approve` command permission plus the deciders list, never
  from `tools: ["*"]`.
- Natural-language confirmation becomes a gateway-rendered `Confirmation`; keyword tables are
  removed.
- `/approve` and `/deny` remain as typed fallbacks that emit the same decisions.

### 4.6 Worked example: Google edit in a group

Scope policy: no restricted Drive scope is used (`drive`, `drive.readonly`, `drive.metadata*`
require restricted verification and a security assessment). The person signs in with
`openid email` only (non-sensitive) to prove which Google account they are; the owner's service
account, which needs no consent-screen verification, does all Drive work.

```
step check_login    -> Needs(Connect(google))                     if no verified email
step check_sa_share -> Needs(ExternalAction("share with <sa> as editor", verify=check_sa_share))
                                                                  if the service account cannot open the file
step check_access   -> Refused("you have no edit access to this file")
                       unless permissions.list (by the service account) shows the person's email
                       with role writer or owner; group and domain grants cannot be matched by
                       email and are refused (fail closed)
step check_room     -> Needs(Approval(deciders = room members whose verified email has writer
                                      or owner on the file))      if group
step edit           -> re-run check_access immediately before acting, then Done(result),
                       executed with the service account and logged with the requesting email
```

Public "anyone with the link" files are read through `fetch` without login; editing always goes
through this flow. The core never learns Google rules; it runs the flow, raises requirements,
validates decisions. A future option is the user's own token with the non-sensitive
`drive.file` scope and the Google Picker, once a web page for the picker exists.

## 5. Configuration

- `engine.approval.*` modes (`default`, `yolo`, `strict`) remain and decide which
  `approval_class` values raise an `Approval`.
- Generated into `config.yaml` with comments:

```yaml
engine:
  flows:
    # Durable waits: approvals, account links, "please share this file", missing settings.
    default_expiry: 24h      # A request nobody answers expires after this and is reported as expired.
    poll_seconds: 5          # How often the flow worker looks for requests that can continue.
  access:
    access_notice_hours: 24  # In a private chat, a person without access is told so at most once
                             # in this many hours. Groups never receive access refusals.
```

## 6. Metrics and gates

- Zero regex or keyword parsing of mechanism decisions in core or gateways (selfcheck rule).
- Flow resume after restart: no duplicated side effect in the lifecycle tests.
- Every decision path has a test for a non-decider, an expired flow, and a subject-veto case.

## 7. Acceptance criteria

- A reply that merely mentions `/approve #5` is never treated as an approval.
- A `settings get` never requires approval.
- An administrator cannot approve another person's disclosure.
- The Google flow of §4 runs end to end in tests with a fake Google service.
- Restarting mid-flow resumes at the awaited requirement.

## 8. Risks and mitigations

- **Runner grows into a workflow engine.** Mitigation: the scope limits of §4 are part of the
  decision; new requirement kinds need an ADR.
- **Gateways lag behind new kinds.** Mitigation: the SDK text fallback renders every kind.
- **Migration of open approvals.** Mitigation: pre-release; pending approvals are expired at
  migration time.

## 9. Interaction with other ADRs

- ADR-0003: reuses checkpoints, outbox, and action ids.
- ADR-0006: RBAC and approval modes remain; routing and authority change per §4.
- ADR-0010: per-gateway UI completeness now targets requirement kinds.
- ADR-0012: receipts use `speaker=system_receipt` with `actor`.
- ADR-0014: plugins declare flows, requirements, verifiers, and deciders.

## 10. Code references

- `monoagens/gateways/base.py` (`APPROVAL_BADGE`, `approval_id`, `ui_of`),
  `monoagens/gateways/whatsapp/gateway.py` (`send_text`, `remember_approval`).
- `monoagens/engine/access/approvals.py`, `monoagens/engine/commands/approve.py`,
  `monoagens/engine/guard/policy.py`, `monoagens/engine/tools/settings.py`.
- `monoagens/engine/turn/approval_flow.py`, `monoagens/engine/turn/confirmations.py`,
  `monoagens/engine/character/lexicon.py`.
- `plugins/google/tools/google.py`, `plugins/google/google_workspace/`.

*This ADR lives at `docs/v0.1.0/adr/engine/0013-flow-protocol-requirements-and-approvals.md`.*
