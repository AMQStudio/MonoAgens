# MonoAgens Research Round 2 — R-44 & R-45 (Identity Across Bodies & The Plugin Boundary)

**Date:** 2026-11-16  
**Scope:** Part C (Gateway / Waist 2 / Layer above the Core) of [researcher-prompt-2.md](../../research/researcher-prompt-2.md)  
**Target invariants:** [ARCHITECTURE.md](../../ARCHITECTURE.md), [protocol/spec.md](../../openspec/changes/core-and-lab/specs/protocol/spec.md), [gateway/spec.md](../../openspec/changes/bootstrap-vertical-slice/specs/gateway/spec.md)

---

# R-44 One person across bodies

**Status:** answered (core isolation & bilateral-confirmation rule settled by standards, record-linkage decision theory, and security literature; UX timing parameters and retroactive history-unification policy remain to be evaluated in the Lab)

## Answer

Every account reported by a body MUST default in the Gateway to an isolated, deterministic, opaque actor ID derived strictly from `(body_id, platform_account_id)` with zero automatic heuristic merging across platforms; cross-body merging into a shared `ActorId` MUST occur only via **(Tier 0) explicit owner configuration** or **(Tier 1 / Tier 2) bilateral cryptographic or out-of-band challenge-response confirmation** recorded as an append-only Gateway event (`actor_linked`) so the Gateway holds zero hidden state across restarts.

### 1. Platform Identity Proof Classification Hierarchy

A body adapter at Waist 2 MUST declare the strongest verifiable identity proof tier available for the reported `platform_account_id`, never treating mutable handles or display names as principal identifiers:

| Proof Tier | Mechanism & Verified Standards (as of Nov 2026) | What It Proves | Safe for Cross-Body Auto-Merge? |
|---|---|---|---|
| **Tier 0: Owner Explicit Config** (`owner_config`) | Static `[[person]]` mapping in owner-controlled `monoagens.toml` (`bootstrap-vertical-slice/specs/gateway/spec.md`). | Authoritative binding set by the soul's owner (Absolute Line 1). | **Yes** (explicit owner decree). |
| **Tier 1a: Hardware/Passkey Cryptographic Proof** (`crypto_webauthn`) | **W3C WebAuthn Level 3** (W3C Recommendation, 25 Aug 2026; `BE`/`BS` flags distinguish device-bound vs cloud-synced passkeys) & **NIST SP 800-63-4** AAL3 (Aug 2025). | Possession of private key signing a fresh Gateway nonce (`challenge`, `origin`, `rpId`). | **Yes, via challenge-response** (proves same credential/user enrolled both bindings). |
| **Tier 1b: Verifiable Credential / DID Proof** (`crypto_vc_did`) | **W3C Verifiable Credentials Data Model v2.0** (W3C Rec, 15 May 2025) + **DIDs v1.0** (W3C Rec, 19 Jul 2022; **v1.1** CR 05 Mar 2026). | Holder controls DID private key via signed Verifiable Presentation over fresh nonce; issuer attests claims. | **Yes, via challenge-response** + trusted issuer verification. |
| **Tier 1c: Federated OIDC Token** (`crypto_oidc`) | **OpenID Connect Core 1.0** (errata set 2, Dec 2023) + **OAuth 2.0 Security BCP (RFC 9700 / BCP 240)** (Jan 2025): PKCE mandatory, no implicit/ROPC, exact redirect URI, `state` + `nonce`. | Exact `(iss, sub)` within one OpenID Provider when `subject_type = public` (or shared `sector_identifier_uri`). **Pairwise `sub` cannot be linked across distinct RPs.** Cross-IdP `email`/`phone_number` requires `email_verified == true` / `phone_number_verified == true` **plus** active re-authentication of the existing account to block pre-hijacking (Sudhodanan & Paverd 2022; Luo et al. 2026). | **Only within same `(iss, sub)` under active session binding**; NEVER on unverified email/phone claims. |
| **Tier 1d: E2EE Key Continuity / Cross-Signing** (`crypto_e2ee`) | **Matrix v1.19** (08 Jul 2026) Cross-Signing (Ed25519 Master/Self-Signing/User-Signing keys + SAS/QR verification; `mautrix` `cross-signed-verified` / `cross-signed-tofu` bridge trust); **Signal Protocol** Curve25519 long-term identity key + 60-digit Safety Numbers (SHA-512 iterated fingerprint; Signal usernames Feb 2024 decouple routing from phone numbers). | Cryptographic identity key continuity **within** that E2EE protocol/bridge domain (and verified device trust). | **Within-network continuity only**; cross-body linking to another platform still requires bilateral challenge-response. |
| **Tier 1e: Authenticated Email Alignment** (`crypto_email_dmarc`) | **DKIM** (RFC 6376) + **SPF** (RFC 7208) + **DMARC** (RFC 9989, May 2026, obsoleting RFCs 7489 & 9091) aligned `pass` on `From:` domain. *(Chat/email-only rule).* | Sending mail server was authorized by `From:` domain; raw `From:` without aligned DKIM/DMARC `pass` is trivially spoofable. | **No auto-merge** (domain authentication ≠ person identity across chat platforms; usable only as one leg of bilateral challenge-response). |
| **Tier 2: Platform-Asserted Account ID** (`platform_id`) | Authenticated platform API/bot connection reporting immutable platform-scoped ID: Telegram 52-bit numeric `User.id`, Discord 64-bit `Snowflake` string, Slack workspace `user_id` / `enterprise_user`, Matrix `mxid`, WhatsApp `PN-JID` (`@s.whatsapp.net`) or privacy-preserving `LID-JID` (`@lid`, hides phone number in groups/channels). | Consistent, immutable principal **inside one `(body_id, platform_account_id)` tuple**. Note: a platform cannot cryptographically attest ownership of an account on a *different* platform; WhatsApp `LID` intentionally prevents phone-number correlation across contexts. | **Never auto-merge across bodies.** Valid endpoint for bilateral out-of-band confirmation. |
| **Tier 3: Unverified Claims** (`unverified_claim`) | Display names, profile bios, mutable `@username` handles (Telegram/Discord/Signal), unaligned email `From:`, `email_verified != true`, or natural-language claims ("I am Alice from Telegram"). | Zero identity assurance. Vulnerable to impersonation, handle squatting after rename, and account pre-hijacking. | **Strictly forbidden.** Adapters MUST NOT emit mutable handles as `platform_account_id`. |

### 2. Decision-Theoretic Basis (Fellegi–Sunter 1969) & Security Threat Model

In **Fellegi–Sunter (1969)** record linkage, comparing two account records $(r_A, r_B)$ yields a comparison vector $\gamma = (\gamma_1, \dots, \gamma_K)$ and composite log-likelihood ratio:

$$\Lambda(r_A, r_B) = \sum_{k=1}^{K} \log \frac{P(\gamma_k \mid M)}{P(\gamma_k \mid U)}$$

where $M$ is the matched-entity class and $U$ is the unmatched-entity class. Fellegi & Sunter partition decisions into Link ($A_1: \Lambda \ge T_\mu$), Clerical/Interactive Review ($A_2: T_\lambda < \Lambda < T_\mu$), and Non-Link ($A_3: \Lambda \le T_\lambda$) to bound Type I false-match error $\mu = P(A_1 \mid U)$ and Type II false-non-match error $\lambda = P(A_3 \mid M)$. Under Bayesian decision theory (Christen 2012; Binette & Steorts 2022), merging is Bayes-optimal iff expected false-merge cost is below missed-merge cost:

$$C_{\text{I}} \cdot (1 - P(M \mid \gamma)) \le C_{\text{II}} \cdot P(M \mid \gamma) \iff P(M \mid \gamma) \ge \tau_{\text{merge}} = \frac{C_{\text{I}}}{C_{\text{I}} + C_{\text{II}}}$$

In MonoAgens, $C_{\text{I}}$ and $C_{\text{II}}$ are radically asymmetric:
- **Type I cost ($C_{\text{I}}$, false merge):**
  1. **Privilege Escalation (breaches Absolute Line 1):** `origin_actor` is inherited along every causal chain (`core-and-lab/specs/cognitive-loop/spec.md`). Merging an attacker account $B$ into the owner's `ActorId` lets $B$ originate `identity_changing` actions and bypass owner/role gates (`R-41`), and injects the owner's `teacher_weight` and reliability prior $(\alpha_0, \beta_0)$ via `actor_facts` (poisoning `bounded_learning` and epistemic credence!).
  2. **Private Audience Leak (breaches Absolute Line 2):** Privacy filtering checks `context_audience ⊆ item_audience`. Merging attacker $B$ into victim $P_{\text{alice}}$ includes $B$ in `P_alice`'s audience checks, leaking Alice's private memories, expectations, and `PerceptionLedger` cards across bodies.
  3. **Theory-of-Mind Corruption:** `sources` ledger cards conflate what $A$ witnessed in Body 1 with what attacker $B$ asked in Body 2.
  Because Absolute Lines 1 & 2 are hard invariants, **$C_{\text{I}} \to \infty \implies \tau_{\text{merge}} \to 1$**.
- **Type II cost ($C_{\text{II}}$, missed merge):** Two unlinked accounts stay isolated as distinct opaque IDs $(P_A, P_B)$, incurring only reversible cross-body recall/closeness fragmentation until explicit linking occurs.

Empirical security studies confirm that real-world heuristic or email-only account linking fails catastrophically: **Sudhodanan & Paverd (2022)** demonstrated pre-hijacking flaws across dozens of major web services when unverified/pre-created email accounts are merged on federated login, while **Luo et al. (2025, 2026)** uncovered Cross-app OAuth Account Takeover (**COAT**) and linking-CSRF attacks where missing session/state binding in connector ecosystems allows an attacker to hijack linked cross-platform accounts. Therefore, **automatic heuristic merging MUST have $T_\mu = +\infty$ (forbidden)**, and every unverified cross-platform similarity falls into non-link ($A_3$) unless explicitly resolved through owner config or bilateral confirmation ($A_2 \to \text{explicit interactive proof}$).

### 3. Deterministic Gateway Rule & Bilateral Challenge-Response Protocol

1. **Canonical Isolated Fallback (Zero Hidden State, Deterministic Across Restarts):**
   Let $b \in \Sigma^+$ be `body_id`, $a \in \Sigma^+$ be the immutable `platform_account_id` asserted by the adapter (with `proof_tier`), $\mathcal{C}_{\text{person}}$ be owner-configured `[[person]]` tuples in `monoagens.toml`, and $\mathcal{E}_{\text{link}}$ be the projection of `actor_linked` / `actor_unlinked` events from the append-only log (ordered by event sequence). Define length-prefixed canonical key encoding $\kappa(b, a) = \text{u32\_le}(|b|) \parallel b \parallel \text{u32\_le}(|a|) \parallel a$:

   $$\text{ResolveActor}(b, a) = \begin{cases} P_{\text{cfg}} & \text{if } (b, a) \mapsto P_{\text{cfg}} \in \mathcal{C}_{\text{person}} \\ P_{\text{log}} & \text{else if } (b, a) \mapsto P_{\text{log}} \in \mathcal{E}_{\text{link}} \text{ (not revoked by later } \mathtt{actor\_unlinked}\text{)} \\ \mathtt{"per\_"} \parallel \text{hex}_{32}\big(\text{BLAKE3}(\text{"monoagens.actor.v1"} \parallel \kappa(b, a))\big) & \text{otherwise} \end{cases}$$

   - *Why length-prefixed $\kappa(b, a)$:* Fixes delimiter collision in `bootstrap-vertical-slice` (`body:account`) when $b$ or $a$ contains `:` (e.g., Matrix MXIDs `@alice:matrix.org`, `did:web:...`).
   - *Determinism & "Gateway never learns":* Replay over `monoagens.toml` + log yields the identical `ActorId` map with zero external state. The Core treats every returned string as purely opaque and NEVER parses `per_`.

2. **Bilateral Challenge-Response Linking (Runtime):**
   To link unmapped/secondary account $B = (b_2, a_2)$ (currently $P_2$) to canonical $P_1$ tied to $A = (b_1, a_1)$:
   - **Step 1 (Challenge generation & public commitment):** Gateway draws two independent $k_{\text{bits}}$-bit CSPRNG nonces $(N_1, N_2)$ (or cryptographic challenge strings), delivers $N_1$ over body $b_1$ (DM to $a_1$) and $N_2$ over body $b_2$ (DM to $a_2$) with expiry $t_{\text{exp}} = t_{\text{now}} + \Delta_{\text{link}}$, and appends `actor_link_challenge(P_1, b_2, a_2, H(N_1), H(N_2), t_exp)` to the log (storing only cryptographic hashes $H(N_i)$, never plaintext codes, respecting *"Secrets never enter the log"*).
   - **Step 2 (Bilateral proof & privilege guard):** Both sides must confirm within $t \le t_{\text{exp}}$ and within `link_max_attempts`:
     - Side $B$ returns $N_2$ from $(b_2, a_2)$ (or signs the challenge via Tier 1 WebAuthn/VC/OIDC);
     - Side $A$ returns $N_1$ from $(b_1, a_1)$ explicitly approving binding $(b_2, a_2) \to P_1$.
     - **Owner-escalation guard (`owner_escalation_guard = true`):** If $P_1$ holds elevated/owner status (Valve policy `R-41`), linking MUST additionally satisfy local owner confirmation (CLI/owner token or Tier 1 owner WebAuthn credential). Unilateral requests (`$B$` claiming to be `$A$`, or `$A$` trying to pull `$B$` without `$B$` proving possession) are rejected.
   - **Step 3 (Append-only recording & non-retroactive privacy):** Upon bilateral match, Gateway appends `actor_linked(body_id: b_2, account_id: a_2, from_actor: P_2, to_actor: P_1, proof_tier, evidence_digest, confirmed_by, timestamp)`.
     - **Append-only invariant:** Historical log events with `actor = P_2` are **never mutated**.
     - **Prospective mapping:** All subsequent percepts from $(b_2, a_2)$ enter the Core as `actor = P_1` (and inherit `P_1`'s `actor_facts` priors).
     - **Fail-closed audience boundary (`retroactive_merge = false`):** Pre-link events logged under $P_2$ do NOT automatically grant $P_2$'s pre-link conversation context access to $P_1$'s pre-link private items (nor vice versa) unless the owner explicitly invokes a separate declassification/merge rule in the Valve (`R-33`).

### Repository Audit (Contradictions / Gaps Found)
- **`bootstrap-vertical-slice/design.md` (Decision 11):** Specifies `"per_" + hash(body:account)`. Concatenation with `:` is ambiguous when `body_id` or `platform_account_id` contains `:` (e.g., Matrix MXIDs, DIDs, WhatsApp JIDs). Must be replaced with length-prefixed domain-separated canonical hashing $\kappa(b, a)$.
- **`core-and-lab/design.md` (Decision 4) & `bootstrap-vertical-slice`:** Neither change yet includes `actor_link_challenge`, `actor_linked`, or `actor_unlinked` event kinds, nor requires Waist 2 adapters to emit immutable principal IDs + `proof_tier` metadata (leaving out runtime linking under "Gateway never learns"). Note also that `bootstrap-vertical-slice` currently uses static `[[person]]` config only; runtime linking should land cleanly in the Gateway/Policy change.
- **Scope across bodies:** Email DKIM/DMARC rules (`crypto_email_dmarc`) and chat handles/LIDs apply to messaging bodies; embodied devices (robots, local sensors) authenticate at the **hardware/device principal level** (TLS/mTLS client cert, hardware serial, or owner-configured `device_actor` in `[[person]]`) rather than user chat accounts.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `heuristic_merge_enabled` | Allow probabilistic/heuristic cross-body auto-merging | `false` ($T_\mu = +\infty$) | `{false}` (invariant-locked; Lab ablation only `[0.5, 1.0]`) | Fellegi & Sunter (1969) with $C_{\text{I}} \to \infty$ under Absolute Lines 1 & 2 |
| `link_challenge_ttl_s` | Expiry window for bilateral linking challenges | `600 s` | `[60 s, 3600 s]` | Start value (short-lived OAuth/verification nonce practice); tune in Lab |
| `link_max_attempts` | Max failed challenge echoes before challenge invalidation | `3` | `[1, 5]` | Start value; tune in Lab |
| `challenge_entropy_bits` | CSPRNG entropy per channel challenge token | `128 bits` (or 8-char base32 human-readable out-of-band SAS if typed manually: $\ge 40\text{ bits}$ paired with rate limit) | `[40 bits, 256 bits]` | Start value; NIST SP 800-63-4 secret-entropy alignment |
| `retroactive_merge` | Whether linking $P_2 \to P_1$ unifies pre-link private audiences / ledger cards | `false` | `{false, true}` (owner opt-in only) | Fail-closed default from Absolute Line 2 |
| `owner_escalation_guard` | Require explicit owner-tier confirmation before linking any account to an elevated/owner `ActorId` | `true` | `{true}` (invariant-locked) | Absolute Line 1 ([ARCHITECTURE.md](../../ARCHITECTURE.md)) |

## Neutral value

`heuristic_merge_enabled = false`, `retroactive_merge = false`, and no runtime `actor_linked` events (Gateway resolves static `[[person]]` config and assigns deterministic isolated `per_<blake3(b, a)>` IDs to all other accounts). This equals the `bootstrap-vertical-slice` baseline; the single-queue loop runs across all bodies with zero crashes or cross-body leakage.

## Claim

Enforcing isolated deterministic `per_` IDs by default, prohibiting heuristic auto-merging, requiring immutable principal IDs from adapters, and allowing cross-body linking solely via owner config or bilateral challenge-response logged as events achieves:
1. **Zero Type I false-merge security breaches** (`false_merge_rate = 0`, `unauthorized_identity_change = 0`, `private_audience_leak = 0`, `teacher_weight_poisoning = 0`) against display-name spoofing, handle squatting, unverified email/OIDC claims, and unilateral linking CSRF; while
2. **Full Type II cross-body continuity** after explicit or bilateral linking (`closeness`, `PerceptionLedger` false-belief tracking, and cross-body promise continuity remain intact across Gateway restarts via log replay alone).

## How the Lab would test it

1. **Adversarial Type I (False-Merge & Pre-Hijack) Suite:** Construct a multi-body scenario (`TestBody_A` Telegram-like, `TestBody_B` Discord-like, `TestBody_C` Email-like) where an attacker on `TestBody_B` adopts the exact display name and mutable handle of the Owner / Alice on `TestBody_A`, sends a spoofed unaligned `From:` email on `TestBody_C`, and sends a unilateral link request. Assert:
   - Distinct opaque `ActorId` from Victim/Owner;
   - `private_audience_leak_count == 0` (measured via Lab's existing narrower-audience leak metric in [lab/spec.md](../../openspec/changes/core-and-lab/specs/lab/spec.md#L144-L145));
   - `unauthorized_identity_change_count == 0`;
   - `teacher_weight` & `reliability` of Victim/Owner unchanged (`0` poisoning).
2. **Bilateral Challenge-Response & Restart Replay Suite:** Alice starts unmapped on `TestBody_A` ($P_1$) and `TestBody_B` ($P_2$), executes bilateral challenge-response ($N_1$ on A, $N_2$ on B) producing `actor_linked`, then Gateway restarts (dropping all in-memory maps and reloading from `log.db` + config). Assert:
   - Subsequent percepts from both bodies resolve deterministically to $P_1$;
   - `PerceptionLedger` card on $P_1$ tracks what Alice witnessed on A when asked on B (`false_belief_accuracy` improves vs unlinked ablation);
   - Pre-link $P_1$ private items remain inaccessible to any separate $P_2$-only historical audience unless `retroactive_merge = true`.
3. **Immutable ID vs Mutable Handle Churn Suite:** Simulate a user changing mutable `@handle` / display name 5 times while `platform_account_id` stays constant (and simulate a WhatsApp-style `PN-JID` vs `LID-JID` group split). Assert `ActorId` stability across renames and clean non-merging when only an unlinkable `LID` is visible.

## Established / inferred / open

- **Established:**
  - Fellegi–Sunter (1969) optimal linkage threshold $\tau = C_{\text{I}} / (C_{\text{I}} + C_{\text{II}})$ (`10.1080/01621459.1969.10501049`); Bayesian cost-sensitive record linkage frameworks (Christen 2012; Binette & Steorts 2022).
  - Cryptographic authentication & federation standards as of Nov 2026: WebAuthn Level 3 (W3C Rec, 25 Aug 2026), W3C VC 2.0 (W3C Rec, 15 May 2025), W3C DID Core v1.0 (W3C Rec, 19 Jul 2022; v1.1 CR 05 Mar 2026), NIST SP 800-63-4 (Aug 2025), OIDC Core 1.0 (`sub` provider-scoped, pairwise `sub` non-linkable across RPs, `email_verified`/`phone_number_verified` flags), OAuth 2.0 Security BCP RFC 9700 (Jan 2025), DKIM RFC 6376 / SPF RFC 7208 / DMARC RFC 9989 (May 2026), Matrix v1.19 cross-signing & `mautrix` verification levels, Signal Curve25519 safety numbers & phone-privacy usernames (Feb 2024).
  - Empirical vulnerabilities of heuristic/unverified account linking: pre-hijacking (Sudhodanan & Paverd 2022, USENIX Security), cross-app OAuth takeover COAT/CORF (Luo et al. 2025, USENIX Security), and connector OAuth account-linking exploits (Luo et al. 2026, IEEE S&P).
- **Inferred:**
  - Mapping MonoAgens's Absolute Lines 1 & 2 into Fellegi–Sunter's cost ratio ($C_{\text{I}} \to \infty \implies T_\mu = +\infty$, zero automatic heuristic merge).
  - The two-sided challenge-response event log schema (`actor_link_challenge` storing commitment hashes $H(N_i)$, `actor_linked`, `actor_unlinked`), length-prefixed $\kappa(b, a)$ canonical encoding, and fail-closed `retroactive_merge = false` policy preserving "Gateway never learns" and log immutability.
- **Open (not settled by the literature):**
  - Optimal `link_challenge_ttl_s` and UX ergonomics for cross-platform bilateral confirmation (e.g., short SAS emoji/code vs clickable signed URL across CLI, chat, and mobile bodies) — requires usability/Lab measurement.
  - Whether post-link **opt-in** ledger/closeness consolidation over pre-link `P_2` history yields measurable recall/ToM benefit without violating privacy expectations when both `P_1` and `P_2` were legitimately the same user prior to linking (test in Lab comparing `retroactive_merge = false` vs owner-authorized projection aliasing).

## Sources

- **[PR]** Fellegi, I. P., & Sunter, A. B. (1969). *A Theory for Record Linkage*. Journal of the American Statistical Association, 64(328), 1183–1210. DOI: `10.1080/01621459.1969.10501049`.
- **[PR]** Christen, P. (2012). *Data Matching: Concepts and Techniques for Record Linkage, Entity Resolution, and Duplicate Detection*. Springer. DOI: `10.1007/978-3-642-31164-2`.
- **[PR]** Binette, O., & Steorts, R. C. (2022). *(Almost) all of entity resolution*. Science Advances, 8(12). DOI: `10.1126/sciadv.abi8021`.
- **[PR]** Sudhodanan, A., & Paverd, A. (2022). *Pre-hijacked accounts: An Empirical Study of Security Failures in User Account Creation on the Web*. 31st USENIX Security Symposium (USENIX Security 22), pp. 1795–1812. URL: https://www.usenix.org/conference/usenixsecurity22/presentation/sudhodanan
- **[PR]** Luo, K., Wang, X., Fung, P. H. A., Lau, W. C., & Lecomte, J. (2025). *Universal Cross-app Attacks: Exploiting and Securing OAuth 2.0 in Integration Platforms*. 34th USENIX Security Symposium (USENIX Security 25). URL: https://www.usenix.org/conference/usenixsecurity25/presentation/luo
- **[PR]** Luo, K., Wang, X., Fung, P. H. A., & Lau, W. C. (2026). *Demystifying the (In)Security of OAuth-Based Account Linking in Connector Ecosystems*. 47th IEEE Symposium on Security and Privacy (IEEE S&P 2026). DOI: `10.1109/sp63933.2026.00128`.
- **[IND]** NIST (2025). *NIST Special Publication 800-63-4: Digital Identity Guidelines* (August 2025). DOI: `10.6028/nist.sp.800-63-4`. URL: https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-63-4.pdf
- **[IND]** W3C (2026). *Web Authentication: An API for accessing Public Key Credentials - Level 3* (W3C Recommendation, 25 August 2026). URL: https://www.w3.org/TR/2026/REC-webauthn-3-20260825/
- **[IND]** W3C (2025). *Verifiable Credentials Data Model v2.0* (W3C Recommendation, 15 May 2025). URL: https://www.w3.org/TR/2025/REC-vc-data-model-2.0-20250515/
- **[IND]** W3C (2022 / 2026). *Decentralized Identifiers (DIDs) v1.0* (W3C Recommendation, 19 July 2022, https://www.w3.org/TR/did-1.0/) & *DIDs v1.1* (W3C Candidate Recommendation Snapshot, 05 March 2026, https://www.w3.org/TR/did-1.1/).
- **[IND]** OpenID Foundation (2023). *OpenID Connect Core 1.0 incorporating errata set 2* (15 December 2023). URL: https://openid.net/specs/openid-connect-core-1_0.html
- **[IND]** IETF (2025). Lodderstedt, T., Bradley, J., Labunets, A., & Fett, D. *Best Current Practice for OAuth 2.0 Security*, RFC 9700 / BCP 240 (January 2025). URL: https://www.rfc-editor.org/rfc/rfc9700.txt
- **[IND]** IETF Email Authentication RFCs: Crocker, D. & Hansen, T. *DomainKeys Identified Mail (DKIM) Signatures*, RFC 6376 (September 2011, https://www.rfc-editor.org/rfc/rfc6376.txt); Kitterman, S. *Sender Policy Framework (SPF)*, RFC 7208 (April 2014, https://www.rfc-editor.org/rfc/rfc7208.txt); Kitterman, S. *Domain-Based Message Authentication, Reporting, and Conformance (DMARC)*, RFC 9989 (May 2026, obsoletes RFCs 7489 & 9091, https://www.rfc-editor.org/info/rfc9989/).
- **[IND]** Matrix.org Foundation (2026). *Matrix Specification v1.19* (08 July 2026, Client-Server API: Cross-signing & Key Verification Framework). URL: https://spec.matrix.org/v1.19/client-server-api/ ; Mautrix End-to-Bridge Encryption Docs: https://docs.mau.fi/bridges/general/end-to-bridge-encryption.html
- **[IND]** Signal Messenger LLC (2016 / 2024 / 2026). *Signal Specifications* (https://signal.org/docs/), *Safety number updates* (17 Nov 2016, https://signal.org/blog/safety-number-updates/), *Keep your phone number private with Signal usernames* (20 Feb 2024, https://signal.org/blog/phone-number-privacy-usernames/).
- **[IND]** Platform Principal Identifier Docs: Telegram Bot API `User.id` (https://core.telegram.org/bots/api), Discord API Reference — Snowflakes (https://discord.com/developers/docs/reference), Slack User Object (https://api.slack.com/types/user). *(Note: WhatsApp Linked IDs `@lid` vs `@s.whatsapp.net` PN-JID behavior is documented in third-party protocol reverse-engineering docs and search summaries; Meta's internal WhatsApp LID spec is not publicly published as an open RFC.)*

---

# R-45 The plugin boundary

**Status:** answered (state of all 6 external protocols verified as of November 2026; native JSON-RPC 2.0 Waist-2 contract + MCP/A2A/ROS-2 adapter plugins settled by feature-by-feature gap analysis; high-frequency timeout/batching thresholds remain to be profiled in Lab)

## Answer

MonoAgens SHOULD define **Waist 2 (`monoagens.body.v1`) as a small, bidirectional JSON-RPC 2.0 subprocess/socket protocol** combining **LSP's wire framing, capability handshake, and additive versioning** with **DAP's unsolicited event stream** and **A2A/ROS-2's typed terminal state machine**, rather than forcing bodies to be raw MCP servers; pure stateless external tool servers MUST instead be wrapped by an official **MCP client bridge plugin** (`plugin-mcp-bridge`) speaking MCP `2026-07-28` outward and Waist 2 inward.

### 1. State of the Six Protocol Ecosystems (Verified as of November 2026)

1. **Model Context Protocol (MCP) — Revision `2026-07-28`**
   - **Governance:** Contributed by Anthropic to the Linux Foundation's **Agentic AI Foundation (AAIF)** on December 9, 2025. Revisions: `2024-11-05` $\to$ `2025-03-26` (Streamable HTTP, OAuth) $\to$ `2025-06-18` (structured tool output, Elicitation) $\to$ `2025-11-25` (experimental Tasks, URL elicitation) $\to$ **`2026-07-28`** (current).
   - **Architecture & 2026 Changes:** JSON-RPC 2.0 (UTF-8, non-null `string | number` IDs) over `stdio` (NDJSON `\n`-delimited) or `Streamable HTTP` (single POST `/mcp` returning JSON or request-scoped SSE; legacy HTTP+SSE formally Deprecated; SSE `Last-Event-ID` resumability removed). Auth uses OAuth 2.1 draft, Protected Resource Metadata (`RFC 9728`), Resource Indicators (`RFC 8707`), and Client ID Metadata Documents (deprecating `RFC 7591` DCR). Crucially, **`2026-07-28` made MCP stateless**: it **removed `initialize`/`notifications/initialized` and `Mcp-Session-Id`**, placing version/capabilities in per-request `_meta` (`io.modelcontextprotocol/protocolVersion`, `io.modelcontextprotocol/clientCapabilities`) and `MCP-Protocol-Version` headers plus mandatory `server/discover`. It deprecated **Roots, Sampling, and Logging** (`SEP-2577`), replaced server-initiated requests with **Multi Round-Trip Requests (MRTR)** (`resultType: "input_required"` + client retry with `inputResponses`/`requestState`, `SEP-2322`), replaced HTTP GET + `resources/subscribe` with `subscriptions/listen` (POST-response stream strictly for `toolsListChanged`, `promptsListChanged`, `resourcesListChanged`, `resourceSubscriptions`), and moved **Tasks** into optional extension `io.modelcontextprotocol/tasks` (`SEP-2663`: `CreateTaskResult`, polling `tasks/get`, `tasks/update`, `tasks/cancel`; statuses `{working, input_required, completed, failed, cancelled}`). Even over `stdio`, **a server MUST NOT send JSON-RPC requests to the client**.
2. **Agent2Agent (A2A) — Version `v1.0.1`**
   - **Governance & Merge:** Hosted by Linux Foundation / AAIF. IBM's **Agent Communication Protocol (ACP / BeeAI)** **merged into A2A and archived its repository in August 2025**. A2A reached **`v1.0.0` on March 12, 2026** and **`v1.0.1` on May 28, 2026**.
   - **Architecture & Primitives:** Canonical Protobuf (`lf.a2a.v1.A2AService`) with three bindings: JSON-RPC 2.0 over HTTP(S), gRPC, and HTTP+JSON/REST (`application/a2a+json`). Discovery via signed `AgentCard` (`supported_interfaces`, `skills`, `capabilities`, `security_schemes`). Core operations (`SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask`, `TaskPushNotificationConfig`) manipulate `Message`, `Part`, `Artifact`, and `Task` with `TaskState` $\in$ `{UNSPECIFIED, SUBMITTED, WORKING, COMPLETED, FAILED, CANCELED, INPUT_REQUIRED, REJECTED, AUTH_REQUIRED}`.
3. **Agent Client Protocol (ACP — Zed / JetBrains) — `v1` (`schema-v1.32.0`) & draft `v2` (`schema-v2.0.0-alpha.16`)**
   - *(Distinct from IBM's archived Agent Communication Protocol).* Introduced by Zed (Aug 2025) and JetBrains (Oct 2025), with an official ACP Registry (early 2026) and Rust/TS/Python/Kotlin/Java SDKs (`schema-v1.32.0` / Rust crate `v1.18.0` and `schema-v2.0.0-alpha.16` published Nov 14, 2026).
   - **Architecture & Primitives:** JSON-RPC 2.0 (non-null IDs, no batches in v2) over `stdio` subprocesses (HTTP/WebSocket RFD in progress). Designed for IDE (Client) $\leftrightarrow$ Coding Agent (Server): `initialize`, `session/new`, `session/resume`, `session/prompt`, `session/update` notifications (`tool_call`, `tool_call_update`, `plan_update`, `state_update`), `session/cancel`, and `session/request_permission`. In v2, `session/prompt` returns immediately upon acceptance (stop reason moves to `state_update`), and client `fs/*` / `terminal/*` methods were removed in favor of passing MCP servers to the agent.
4. **OpenAI Tool Calling & Responses API (2025–2026)**
   - Launched March 2025; as of Nov 2026 supports stateful server-side chaining (`previous_response_id`), asynchronous **`background: true`** execution (with `store: true` or ~10-min transient retention), persistent **WebSocket mode** (`/v1/responses` with `stream_id` multiplexing), and built-in remote **MCP connectors** (`type: "mcp"`, `require_approval`, `mcp_approval_request` / `mcp_approval_response`). Proprietary model-provider API, not a generic local plugin IPC boundary.
5. **Language Server Protocol (LSP `3.18.0`) & Debug Adapter Protocol (DAP `v1.71.0`)**
   - **LSP `3.18.0`** (changelog `06/04/2026` = June 4, 2026): JSON-RPC 2.0 over `Content-Length` HTTP-header framed stdio/pipes/sockets; explicit `initialize` $\to$ `initialized` $\to$ `shutdown` $\to$ `exit` lifecycle; bidirectional capability flags (`@since 3.x` additive versioning); `$/cancelRequest` and `$/progress`.
   - **DAP `v1.71.0`** (tagged Feb 6, 2026): `Content-Length` framed JSON using a **custom wire envelope** (`ProtocolMessage { seq, type: "request" | "response" | "event" }`), NOT JSON-RPC 2.0; pairs request/response with unsolicited server-to-client `event` push notifications (`stopped`, `output`, `continued`) and reverse host requests (`runInTerminal`).
6. **ROS 2 (Robot Operating System 2 — Lyrical Luth LTS, May 2026)**
   - Twelfth release **Lyrical Luth** (May 2026, LTS to May 2031; following Kilted Kaiju May 2025 and Jazzy Jalisco May 2024 LTS). Abstract `rmw` middleware over DDS or Zenoh (`rmw_zenoh_cpp`) using CDR binary IDL serialization. Three primitives: **Topics** (QoS-governed async pub/sub streams), **Services** (req/rep RPC), and **Actions** (goal service + cancel service + feedback/status topics + result service) driven by `action_msgs/msg/GoalStatus`: `STATUS_UNKNOWN (0)`, `STATUS_ACCEPTED (1)`, `STATUS_EXECUTING (2)`, `STATUS_CANCELING (3)`, `STATUS_SUCCEEDED (4)`, `STATUS_CANCELED (5)`, `STATUS_ABORTED (6)`.

### 2. Feature-by-Feature Matrix Against MonoAgens Waist 2's Six Requirements

| Waist 2 Requirement | MCP (`2026-07-28`) | A2A (`v1.0.1`) | ACP (Zed/JB `v1`/`v2`) | OpenAI Responses API | LSP `3.18` / DAP `1.71` | ROS 2 (`Lyrical` LTS) | **Native Waist 2 (`monoagens.body.v1`)** |
|---|---|---|---|---|---|---|---|
| **1. Pushed percepts with `(actor, place, audience)`** | **Missing.** Pull/RPC only; `subscriptions/listen` pushes URI invalidation hints only (`resources/updated`), no `(actor, place, audience)` or arbitrary world events. | **Partial / Misaligned.** Streaming/webhooks exist only inside client-initiated `Task`s; no unsolicited world percepts or `place`/`audience` witnessing model. | **Partial / Inverted.** Agent pushes `session/update` notifications to IDE Client, scoped to a coding prompt turn, not `(actor, place, audience)`. | **Missing.** Request-driven model API; WebSocket mode streams model output per response, not external environment percepts. | **LSP:** notifications (`publishDiagnostics`); **DAP:** unsolicited async `event` stream (pattern match, IDE-only schema). | **Topics** provide native QoS-controlled continuous sensory push; lacks conversational `actor` & `audience` privacy set. | **First-class** `body/percept` pushed anytime with mandatory `(actor, place, audience)` + optional reportable facts. |
| **2. Capability descriptors `(places, latency, effect channel/window/noise)`** | **Missing.** `Tool` has `inputSchema`/`outputSchema`/`ToolAnnotations` (`readOnlyHint` etc., explicitly untrusted); no `places`, `typical_latency`, `communicative`, `identity_changing`, or `effect`. | **Partial.** `AgentSkill` has `id`, `name`, `description`, `input_modes`/`output_modes`; lacks `places`, `typical_latency`, policy flags, and sensor `effect`. | **Inverted.** Negotiates IDE capabilities, not body effectors/sensors. | **Missing.** Model tool schema (`parameters`) only. | **Capability negotiation** at `initialize` (pattern match), domain-locked to editor/debugger features. | **Static IDL** (`.srv`/`.action`), no dynamic LLM-facing descriptions, `places`, or effect noise priors. | **First-class** `CapabilityDescriptor` registered per owned `place` with flags, `typical_latency_ms`, and `effect`. |
| **3. Exactly-one receipt $\in$ `{done, refused, held, failed, unknown}`** | **Partial / Lossy.** Base `tools/call` has JSON-RPC error or `isError: bool` (collapses `refused`/`failed`/`unknown`). `ext-tasks` has `{working, input_required, completed, failed, cancelled}` via polling; lacks distinct `refused` & `unknown`. | **Closest semantic match.** `TaskState`: `COMPLETED` $\to$ `done`, `REJECTED` $\to$ `refused`, `INPUT_REQUIRED`/`AUTH_REQUIRED` $\to$ `held`, `FAILED`/`CANCELED` $\to$ `failed`, `UNSPECIFIED`/timeout $\to$ `unknown`. | **Partial.** v2 separates `session/prompt` accept from terminal `state_update` stop reasons + `request_permission`, scoped to prompt turns. | **Misaligned.** `mcp_approval_request` pauses model loop remotely; no body receipt state machine. | **LSP:** single JSON-RPC response or error; **DAP:** response + async `terminated`/`exited` events. | **Strong.** `GoalStatus`: `SUCCEEDED` $\to$ `done`, goal rejection $\to$ `refused`, `ABORTED`/`CANCELED` $\to$ `failed`, `UNKNOWN` $\to$ `unknown`; lacks `held` (consent). | **Strict idempotent FSM** enforced by Gateway (`accepted` optional $\to$ **exactly one** terminal `{done, refused, held, failed, unknown}` within `timeout_ms`). |
| **4. Plugin lifecycle & crash isolation** | **Regressed in 2026-07-28.** Removed `initialize` handshake and sessions to force stateless HTTP microservices; `stdio` still supports subprocesses. | **Remote HTTP/gRPC.** Expects autonomous web services (`AgentCard`), not supervised local subprocesses. | **Strong.** Subprocess `stdio` JSON-RPC + `initialize` + cancellation + ACP Registry. | **N/A** (cloud vendor HTTP/WS). | **Gold standard.** Subprocess `stdio`/socket, `initialize` $\to$ `initialized` $\to$ `shutdown` $\to$ `exit`. | **Distributed nodes.** DDS/Zenoh discovery + ROS 2 Lifecycle Managed Nodes; heavy runtime. | **Supervised subprocess (`stdio`) or local socket**, `body/initialize` $\to$ `body/register` $\to$ `heartbeat`/EOF crash fallback $\to$ `shutdown`/`exit`. |
| **5. Additive versioning** | **Exact date match.** Per-request version list; rejects unsupported with `-32022`. Extensible via `_meta` & `extensions`. | **Strong.** `A2A-Version` header, Protobuf additive field evolution, `VersionNotSupportedError`. | **Strong (v2).** Integer `protocolVersion` negotiation (`-32022`), open enums/unions (`_` prefix). | **API version/endpoints.** Proprietary vendor evolution. | **Strong.** Semver + capability flags; DAP adds JSON schema fields additively. | **IDL/topic hashing.** Breaking `.msg` hash changes fail DDS/Zenoh type matching across releases. | **Semver + MonoAgens rule:** `protocol_version` on envelope; accept older minor, ignore unknown fields (`#[serde(default)]`), reject newer with explicit version error. |
| **6. "An adapter never thinks"** | **Violated by design intent.** MCP tools are explicitly *model-controlled*; `Sampling` (though deprecated) let servers invoke LLMs. | **Violated by design.** A2A peers *are* autonomous thinking agents. | **Violated by design.** ACP Servers *are* LLM coding agents. | **N/A** (is the LLM provider). | **Preserved.** Language/debug servers never run agentic loops. | **Preserved.** Drivers/controllers are deterministic nodes unless user runs planner nodes. | **Invariant-enforced.** Body pushes percepts & executes acts only; Core/Valve owns all appraisal and reasoning. |

### 3. Direct Answer: Could a MonoAgens Body Be an MCP Server Plus a Small Extension?

**No — building a MonoAgens body directly as an MCP `2026-07-28` server plus an MCP extension fails architecturally, losing or breaking five core Waist 2 properties:**
1. **Push-first Perception vs Stateless Request-Response Inversion:** MCP `2026-07-28` explicitly prohibits servers from initiating JSON-RPC requests over `stdout` or HTTP SSE and forces server-to-client needs into MRTR (`InputRequiredResult`) or URI-pre-subscribed `subscriptions/listen` invalidations. A real-world body (chat gateway, IoT sensor bus, game socket) must push unsolicited percepts for newly encountered rooms/places/actors at any time, without waiting for a client poll or pre-known resource URI.
2. **Stateless 2026-07-28 Model vs Stateful Body Ownership:** Removing `initialize` and `Mcp-Session-Id` in `2026-07-28` pushes MCP servers to be stateless microservices. A MonoAgens body maintains persistent platform sockets, owns dynamic `places[]`, reports `body_state`, and must cleanly fail pending actions as `unknown` upon subprocess death.
3. **Loss of Capability Physics & Policy Flags:** Packing `places[]`, `communicative`, `identity_changing`, `typical_latency_ms`, and `effect(channel, window, noise)` into MCP `ToolAnnotations` or `_meta` violates MCP's own warning that `ToolAnnotations` are untrusted UI hints, and standard MCP clients/inspectors cannot route by `place` or enforce Absolute Line 1.
4. **Collapsed Receipt Semantics:** MCP `tools/call` (`isError: bool`) and `io.modelcontextprotocol/tasks` (polling-first `{working, input_required, completed, failed, cancelled}`) lack the semantic split between `refused` (policy/remote denial — do not retry blindly), `failed` (execution failure), and `unknown` (ambiguous timeout — must not blindly replay non-idempotent physical/financial actions), and omit at-most-once terminal receipt enforcement.
5. **`audience` Semantic Collision:** MCP's `annotations.audience` (`["user", "assistant"]`) governs UI display role, whereas MonoAgens's `audience: Vec<ActorId>` is the set of real-world witnesses governing Absolute Line 2 privacy and `PerceptionLedger` theory of mind.

### 4. Formal Specification & Wire Formula for Waist 2 (`monoagens.body.v1`)

Define the Waist 2 boundary contract as a bidirectional JSON-RPC 2.0 protocol over `stdio` (NDJSON, logs on `stderr`) or local Unix domain socket, partitioned into deterministic message transitions and bounded action receipts:

1. **Additive Versioning Rule:** Let every message envelope carry `protocol_version = (M_{\text{msg}}, m_{\text{msg}})` against Gateway-supported `(M_{\text{gw}}, m_{\text{gw}})`:

   $$\text{AcceptEnvelope} = \mathbb{I}\!\left[M_{\text{msg}} = M_{\text{gw}} \;\land\; m_{\text{msg}} \le m_{\text{gw}}\right]$$

   Unknown JSON fields MUST be ignored (`serde` without `deny_unknown_fields`). Any message with $M_{\text{msg}} \ne M_{\text{gw}}$ or $m_{\text{msg}} > m_{\text{gw}}$ fails loudly with JSON-RPC error `-32022` reporting both versions.

2. **Exactly-One Receipt State Machine (Idempotent Terminal Projection):**
   For any dispatched action $a$ with unique `action_id` and configured time limit $T_{\text{act}} = \min(\text{action\_timeout\_ms}, \, \text{body\_limit})$, define receipt emission $R(a)$ as a write-once register transitioning from `Pending` (or `Accepted` / `Held`):

   $$R(a) = \begin{cases} r_{\text{body}} \in \{\mathtt{done}, \mathtt{refused}, \mathtt{held}, \mathtt{failed}\} & \text{if first valid terminal/held response arrives at } t < t_0 + T_{\text{act}} \\ \mathtt{unknown} & \text{if } t \ge t_0 + T_{\text{act}} \text{ or plugin crashes before terminal receipt} \end{cases}$$

   Any duplicate or post-timeout receipt for `action_id` is **dropped and logged as a warning** (`receipt_cardinality(a) == 1` invariant). When `held` resolves later (`R-42` approval/denial), resolution emits a new linked resolution event (`action_resumed` / follow-up chain) rather than violating single-receipt uniqueness for the held dispatch. *(Note: `core-and-lab/specs/protocol/spec.md` treats `held` itself as one of the five terminal receipt outcomes for that action invocation!)*

3. **Concrete Ecosystem Bridging Architecture (No Invention Where Standards Work):**
   - **Native Bodies (`monoagens.body.v1`):** Chat bridges, OS/terminal bodies, sensors, and game adapters speak the lightweight JSON-RPC 2.0 Waist 2 protocol directly (`body/initialize`, `body/register`, `body/percept`, `body/act`, `body/receipt`, `body/state`, `body/shutdown`).
   - **Stateless Tool Leverage (`plugin-mcp-bridge`):** An official Gateway-managed Waist 2 plugin wraps an **MCP `2026-07-28` client** (with dual-era fallback for `2025-11-25`). It translates each configured MCP server into a virtual `place` (`mcp:<server_id>`), exposes `tools/list` as `CapabilityDescriptor`s (`communicative = false`, `identity_changing` defaulted `false` unless owner-flagged in config, `typical_latency_ms` measured/configured), maps `tools/call` + `io.modelcontextprotocol/tasks` + MRTR `input_required` $\to$ `held` into single Waist 2 receipts, and optionally converts `subscriptions/listen` (`notifications/resources/updated`) into `body/percept` events attributed to a configured system `actor` and owner-scoped `audience`.
   - **External Agent & Robot Leverage:** Analogous thin Waist 2 bridge plugins wrap **A2A `v1.0.1`** (`AgentCard` skills $\to$ capabilities; `TaskState` $\to$ 5-way receipt) and **ROS 2 `lyrical`** (Topics $\to$ `body/percept`, `GoalStatus` Actions $\to$ `body/act` + receipt), keeping Core and Gateway 100% platform-agnostic.

### Repository Audit (Contradictions / Gaps Found)
- **`core-and-lab/specs/protocol/spec.md` vs `ARCHITECTURE.md`:** `protocol/spec.md` states *"Every message that crosses a layer boundary SHALL be expressed in these facts [the 6 facts]"*, whereas `ARCHITECTURE.md` defines Waist 2 as *"Actors, places, audiences, percepts, actions and receipts, **lifecycle events, answers and commands**"*. The future Gateway/Waist-2 spec must explicitly clarify that protocol envelopes carry **lifecycle & registration control messages** (`body/initialize`, `body/register`, `body/heartbeat`, `body/shutdown`) alongside the 6 domain facts.
- **`bootstrap-vertical-slice/design.md`:** Left out-of-process plugin IPC as a Non-Goal and did not specify framing or async `accepted` vs terminal `receipt` timeouts over IPC; also note `core-and-lab` Decision 14 uses Chat Completions for `reason` (`openai-compatible`), which remains preferable over OpenAI Responses API's server-side `previous_response_id` state because server-side conversation chaining would bypass MonoAgens's local rebuilt `ContextPlan` and log-projection invariants (`store: false` stateless mode is mandatory if Responses API is ever adapted in `reason`).

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `action_timeout_ms` | Max wait for a body's terminal receipt before emitting `unknown` | `30000 ms` (default; overridable per capability/body via `typical_latency`) | `[1000 ms, 300000 ms]` | Start value (matches typical HTTP/subprocess tool budgets; tune in Lab per body class) |
| `hook_timeout_ms` | Max wait for synchronous/bounded plugin hook calls (`gateway/spec.md`) | `500 ms` | `[50 ms, 5000 ms]` | Start value; tune in Lab to prevent blocking single-consumer queue |
| `plugin_heartbeat_ms` | Health-check ping interval for installed OS-process plugins (`0` = EOF-only) | `15000 ms` | `[0 ms, 60000 ms]` | Start value; tune in Lab |
| `max_percept_queue_per_plugin` | Per-plugin inbound backpressure water-mark before Gateway drops/throttles bursts | `1024 events` | `[64, 16384]` | Start value; tune in Lab for high-rate sensor/game bodies (`R-36`) |
| `mcp_task_poll_interval_ms` | Fallback poll cadence for `plugin-mcp-bridge` when MCP server uses `io.modelcontextprotocol/tasks` without `subscriptions/listen` | `1000 ms` (or server `pollIntervalMs`) | `[250 ms, 30000 ms]` | MCP Tasks extension spec (`pollIntervalMs`); tune in Lab |

## Neutral value

No installed out-of-process plugins or MCP/A2A/ROS-2 bridges active (built-in in-process `TestBody` / `cli` terminal body only, `action_timeout_ms = 30000`, `hook_timeout_ms = 500`, `plugin_heartbeat_ms = 0`). The Core runs identically via synchronous `Core::step` without IPC overhead.

## Claim

Separating Waist 2 into a lightweight bidirectional JSON-RPC 2.0 body contract (`monoagens.body.v1`) paired with adapter bridges (`plugin-mcp-bridge`, etc.) preserves all 6 Waist-2 protocol invariants (pushed percepts with `(actor, place, audience)`, full `CapabilityDescriptor` physics/flags, strict single-receipt `{done, refused, held, failed, unknown}` semantics, crash isolation, additive versioning, and "an adapter never thinks") while achieving full interoperability with unmodified MCP `2026-07-28`, A2A `v1.0.1`, and ROS 2 servers—whereas forcing native bodies into raw MCP servers loses unsolicited perception, place routing, effect predictions, and `refused`/`unknown` receipt precision.

## How the Lab would test it

1. **Out-of-Process Crash & Receipt Cardinality Torture Test:** Spawn a mock out-of-process Waist-2 body subprocess that (a) succeeds on action 1 (`done`), (b) crashes (`SIGKILL`) mid-flight on action 2, (c) hangs past `action_timeout_ms` on action 3 and later tries to send a late `done`, and (d) attempts to push a percept omitting `audience`. Assert:
   - Core queue never stalls (`loop_stall_ms < hook_timeout_ms`);
   - Action 2 & 3 emit **exactly one** `unknown` receipt each; late duplicate receipt on action 3 is rejected;
   - Malformed percept without `audience` is rejected at Waist 2 without entering log;
   - Sibling healthy plugin bodies continue receiving actions uninterrupted.
2. **Additive Version Compatibility Matrix:** Feed serialized Waist-2 messages carrying (a) older supported minor version, (b) current version with extra unknown JSON fields, and (c) newer major/minor version. Assert (a) and (b) parse and execute cleanly, while (c) fails loudly naming both versions.
3. **MCP Bridge Equivalence Test (`plugin-mcp-bridge` vs Raw MCP Failure Modes):** Wrap a reference MCP `2026-07-28` server (supporting `tools/call`, `isError`, MRTR `input_required`, and `io.modelcontextprotocol/tasks`) inside `plugin-mcp-bridge` and run a multi-step Lab scenario alongside a native chat body. Verify end-to-end `place`-scoped routing, `input_required` $\to$ `held` mapping, task completion $\to$ `done` receipt, and zero leakage of tool-level execution state outside the Core's event log.

## Established / inferred / open

- **Established:**
  - Exact current protocol specifications as of November 2026: **MCP `2026-07-28`** (stateless request metadata, `server/discover`, `subscriptions/listen`, MRTR, `io.modelcontextprotocol/tasks` extension, deprecated Roots/Sampling/Logging, governed under Linux Foundation AAIF launched Dec 2025); **A2A `v1.0.1`** (May 2026, merged IBM BeeAI ACP in Aug 2025, Protobuf + JSON-RPC/gRPC/REST, 9-state `TaskState`); **Zed/JetBrains ACP `v1` (`schema-v1.32.0`) & draft `v2` (`schema-v2.0.0-alpha.16`)** (Nov 2026, JSON-RPC 2.0 over `stdio`, IDE $\leftrightarrow$ coding agent); **OpenAI Responses API** (`background: true`, `previous_response_id`, WebSocket mode, remote MCP `mcp_approval_request`); **LSP `3.18.0`** (06/04/2026, JSON-RPC 2.0 `Content-Length`, capability negotiation) & **DAP `v1.71.0`** (Feb 2026, custom `ProtocolMessage` envelope + async events); **ROS 2 Lyrical Luth LTS** (May 2026, DDS/Zenoh Topics/Services/Actions, 7-state `GoalStatus`).
  - Feature gap: none of the six external standards natively supports all six MonoAgens Waist-2 requirements simultaneously, and MCP `2026-07-28`'s stateless/no-server-request shift explicitly forbids server-initiated JSON-RPC requests on `stdout`/HTTP.
- **Inferred:**
  - Concrete design of `monoagens.body.v1` (LSP-style JSON-RPC 2.0 over NDJSON `stdio`/Unix socket + DAP-style unsolicited `body/percept` + idempotent single-receipt FSM) and the adapter-bridge pattern (`plugin-mcp-bridge`, `plugin-a2a-bridge`, `plugin-ros2-bridge`).
  - Exact receipt state mappings between A2A `TaskState`, ROS 2 `GoalStatus`, MCP `ext-tasks`/MRTR, and MonoAgens's `{done, refused, held, failed, unknown}`.
- **Open (not settled by the literature):**
  - Optimal `action_timeout_ms` per body category (chat API vs slow MCP task vs physical robot actuator) and whether `CapabilityDescriptor.typical_latency` should dynamically parameterize Gateway's `unknown` timeout multiplier ($k_{\text{timeout}} \cdot \text{typical\_latency}$) — requires Lab sensitivity sweep alongside `R-36` (high-rate bodies).
  - Backpressure policy (`max_percept_queue_per_plugin`: drop-oldest vs coalesce/summarize at plugin edge vs block) for high-frequency embodied sensors (`R-36`) without violating *"an adapter never thinks"*.
  - Concrete wire serialization for binary media attachments (base64 inline vs out-of-band attachment handle stored by Gateway per `ARCHITECTURE.md` line 105: *"stores attachments"*), to be settled with `R-37`.

## Sources

- **[IND]** Model Context Protocol (2026). *MCP Specification — Revision `2026-07-28`* (28 July 2026): Index, Key Changes (`changelog.md`), Versioning & Compatibility (`basic/versioning.md`), Base Protocol (`basic/index.md`), stdio & Streamable HTTP transports (`basic/transports/stdio.md`, `streamable-http.md`), Subscriptions (`basic/patterns/subscriptions.md`), Multi Round-Trip Requests (`basic/patterns/mrtr.md`), Tools (`server/tools.md`), Resources (`server/resources.md`), Authorization (`basic/authorization/index.md`), and Tasks Extension (`extensions/tasks/overview.md`). URL: https://modelcontextprotocol.io/specification/2026-07-28/
- **[IND]** Linux Foundation (2025). *Agentic AI Foundation (AAIF)* launch (09 Dec 2025; founding projects MCP, goose, AGENTS.md; later hosting A2A and ecosystem projects). Referenced via official MCP docs (https://modelcontextprotocol.io/specification/2026-07-28/) and Linux Foundation announcements.
- **[IND]** Agent2Agent (A2A) Project / Linux Foundation (2026). *Agent2Agent Protocol Specification* (`v1.0.0` released 12 Mar 2026; `v1.0.1` released 28 May 2026; `specification/a2a.proto` `lf.a2a.v1.A2AService`). URL: https://a2a-protocol.org/latest/specification/ and https://github.com/a2aproject/A2A
- **[IND]** Agent Client Protocol (ACP — Zed & JetBrains) (2026). *Agent Client Protocol Docs* (`v1` schema `schema-v1.32.0` / Rust crate `v1.18.0`, and draft `v2` `schema-v2.0.0-alpha.16`, 14 Nov 2026; including *ACP and other protocols* confirming IBM's BeeAI Agent Communication Protocol merged into A2A and archived Aug 2025). URL: https://agentclientprotocol.com/get-started/related-protocols.md , https://agentclientprotocol.com/protocol/v1/overview.md , https://agentclientprotocol.com/protocol/v2/migration.md
- **[IND]** OpenAI (2025–2026). *Responses API Documentation*: Background Mode (https://platform.openai.com/docs/guides/background), WebSocket Mode (https://platform.openai.com/docs/guides/websocket-mode), Remote MCP & Connectors (https://platform.openai.com/docs/guides/tools-connectors-mcp), WebSocket Events Reference (`mcp_approval_request`, https://platform.openai.com/docs/api-reference/responses/websocket-events).
- **[IND]** Microsoft (2026). *Language Server Protocol Specification — 3.18* (`3.18.0`, changelog 06/04/2026). URL: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/
- **[IND]** Microsoft (2026). *Debug Adapter Protocol Specification* (`v1.71.0`, tagged 06 Feb 2026). URL: https://microsoft.github.io/debug-adapter-protocol/specification and https://github.com/microsoft/debug-adapter-protocol
- **[IND]** Open Robotics / ROS 2 (2026). *ROS 2 Lyrical Luth (May 2026 LTS)* Release Notes (https://docs.ros.org/en/rolling/Releases/Release-Lyrical-Luth.html), *ROS 2 Actions Design* (https://design.ros2.org/articles/actions.html), and `action_msgs/msg/GoalStatus.msg` (https://raw.githubusercontent.com/ros2/rcl_interfaces/rolling/action_msgs/msg/GoalStatus.msg).
