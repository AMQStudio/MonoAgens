# Specification — `adapter-readiness`

## ADDED Requirements

### Requirement: Adapters Report Whether They Are Ready to Run
A plugin MAY provide a readiness check as a `status --json` subcommand of its own binary that prints `{"ready": bool, "needs": [..], "detail": ".."}` where each `needs` item is `pairing` or `secret:<ENV_NAME>`; plugins without it are judged by the secrets their arguments name (`--token-env NAME`, `--*-env=NAME`). The gateway SHALL stay platform-neutral: it only runs the check and reads the result.

#### Scenario: WhatsApp not paired
- **WHEN** the WhatsApp plugin has no paired session and its readiness check is run
- **THEN** it reports `ready: false` with `needs: ["pairing"]` without opening a network connection.

### Requirement: Unready Adapters Cannot Be Started or Enabled
The gateway SHALL NOT start a plugin whose readiness check reports not ready (autostart, Start, Restart, or `config.apply` enabling it); the plugin's state SHALL be `NeedsSetup` with the missing items, shown in the sidebar and plugin page as "needs pairing" or "needs <ENV_NAME>" with the action that fixes it (Pair device, or set the secret). Turning "Enable on startup" on SHALL be refused with that reason until the plugin is ready.

#### Scenario: Telegram without a token
- **WHEN** `telegram` is configured with `--token-env TELEGRAM_BOT_TOKEN`, the secret is not set, and the owner presses Start
- **THEN** no process is spawned, the state is "needs TELEGRAM_BOT_TOKEN", and the page offers to set the secret.

#### Scenario: Becomes ready after pairing
- **WHEN** pairing finishes successfully for an enabled adapter in `NeedsSetup`
- **THEN** the readiness check passes and the gateway starts it without restarting MonoAgens.
