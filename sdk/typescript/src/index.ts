import { createHash } from "node:crypto";
import { createInterface } from "node:readline";

export const OPENRPC_SCHEMA_PATH = "../../crates/protocol/openrpc/monoagens.body.v1.openrpc.json";
export const PROTOCOL_VERSION = { major: 1, minor: 0 } as const;
export const WAIST2_METHODS = [
  "body/discover", "body/heartbeat", "body/shutdown", "percept/emit",
  "action/execute", "action/cancel", "action/receipt", "approval/resolve",
  "question/answer", "state/query", "blob/put", "blob/get",
] as const;

export type Waist2Method = (typeof WAIST2_METHODS)[number];
export type RpcId = number | string | null;
export interface ProtocolVersion { major: number; minor: number; }
export interface Waist2Headers {
  idempotency_key: string;
  protocol_version: ProtocolVersion;
  traceparent?: string;
  _meta?: Record<string, unknown>;
}
export interface Waist2Error { code: number; message: string; data?: Record<string, unknown>; }
export interface Waist2Request<P = Record<string, unknown>> {
  jsonrpc: "2.0"; id?: RpcId; method: Waist2Method; params: P;
}
export interface Waist2Response<R = unknown> {
  jsonrpc: "2.0"; id: RpcId; result?: R; error?: Waist2Error;
}
export interface Receipt {
  action_id: string; capability_id: string;
  outcome: "done" | "refused" | "held" | "failed" | "unknown";
  reason_code?: string; detail?: string;
  termination_reason?: "normal" | "canceled_by_core" | "preempted_external" | "timed_out";
  progress?: number; delivered_units?: number; h_act?: string; timestamp_ms: number;
}
export type StateQueryStatus =
  | { state: "in_flight"; progress?: number }
  | { state: "terminal"; receipt: Receipt }
  | { state: "unknown" };

export interface BodyHandlers {
  discover(params: Record<string, unknown>): Promise<Record<string, unknown>> | Record<string, unknown>;
  execute(params: Record<string, unknown>): Promise<{ accepted: boolean; receipt?: Receipt }> | { accepted: boolean; receipt?: Receipt };
  cancel(params: Record<string, unknown>): Promise<{ cancelled: boolean; receipt?: Receipt }> | { cancelled: boolean; receipt?: Receipt };
  resolveApproval(params: Record<string, unknown>): Promise<{ applied: boolean }> | { applied: boolean };
  onPercept?(params: Record<string, unknown>): Promise<{ accepted: boolean }> | { accepted: boolean };
  onQuestion?(params: Record<string, unknown>): Promise<{ accepted: boolean }> | { accepted: boolean };
  onShutdown?(params: Record<string, unknown>): Promise<{ accepted: boolean }> | { accepted: boolean };
}

export class RpcFailure extends Error {
  readonly code: number;
  constructor(code: number, message: string) { super(message); this.code = code; }
}

export const sha256Hex = (bytes: Uint8Array): string =>
  createHash("sha256").update(bytes).digest("hex");

export function verifySha256(expected: string, bytes: Uint8Array): string {
  const actual = sha256Hex(bytes);
  if (actual.toLowerCase() !== expected.toLowerCase()) {
    throw new RpcFailure(-32005, `ERR_HASH_MISMATCH: expected ${expected}, got ${actual}`);
  }
  return actual;
}

export class HeartbeatScheduler {
  private lastSentMs?: number;
  readonly intervalMs: number;
  constructor(intervalMs: number) { this.intervalMs = intervalMs; }
  isDue(nowMs: number): boolean {
    return this.lastSentMs === undefined || nowMs - this.lastSentMs >= this.intervalMs;
  }
  nextRequest(id: RpcId, bodyId: string, status: "ready" | "busy" | "degraded" | "stopped", nowMs: number, key: string): Waist2Request {
    this.lastSentMs = nowMs;
    return {
      jsonrpc: "2.0", id, method: "body/heartbeat",
      params: { headers: { idempotency_key: key, protocol_version: { ...PROTOCOL_VERSION } }, body_id: bodyId, status, timestamp_ms: nowMs },
    };
  }
}

export class BodyAdapter {
  private readonly idem = new Map<string, unknown>();
  private readonly journal = new Map<string, StateQueryStatus>();
  private readonly blobs = new Map<string, Uint8Array>();
  private readonly handlers: BodyHandlers;
  constructor(handlers: BodyHandlers) { this.handlers = handlers; }

  queryState(actionId: string): { action_id: string; status: StateQueryStatus } {
    return { action_id: actionId, status: this.journal.get(actionId) ?? { state: "unknown" } };
  }

  async handleLine(line: string): Promise<string | null> {
    if (!line.trim()) return null;
    const req = JSON.parse(line) as Waist2Request<Record<string, unknown>>;
    const id = req.id ?? null;
    try {
      const result = await this.dispatch(req);
      return JSON.stringify({ jsonrpc: "2.0", id, result } satisfies Waist2Response);
    } catch (err) {
      const code = err instanceof RpcFailure ? err.code : -32003;
      const message = err instanceof Error ? err.message : String(err);
      return JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } } satisfies Waist2Response);
    }
  }

  async runStdio(input = process.stdin, output = process.stdout): Promise<void> {
    const rl = createInterface({ input, crlfDelay: Infinity });
    for await (const line of rl) {
      const out = await this.handleLine(line);
      if (out !== null) output.write(`${out}\n`);
    }
  }

  private async dispatch(req: Waist2Request<Record<string, unknown>>): Promise<unknown> {
    const headers = req.params?.headers as Waist2Headers | undefined;
    const ver = headers?.protocol_version;
    if (!headers?.idempotency_key || !ver || ver.major !== PROTOCOL_VERSION.major || ver.minor > PROTOCOL_VERSION.minor) {
      throw new RpcFailure(-32004, "ERR_UNSUPPORTED_VERSION: unsupported protocol version");
    }
    const key = `${req.method}:${headers.idempotency_key}`;
    if (this.idem.has(key)) return this.idem.get(key);
    let result: unknown;
    switch (req.method) {
      case "body/discover": result = await this.handlers.discover(req.params); break;
      case "body/heartbeat": result = { acknowledged_at_ms: req.params.timestamp_ms }; break;
      case "body/shutdown": result = this.handlers.onShutdown ? await this.handlers.onShutdown(req.params) : { accepted: true }; break;
      case "percept/emit":
        if (!Array.isArray(req.params.audience) || req.params.audience.length === 0) {
          throw new RpcFailure(-32006, "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience");
        }
        result = this.handlers.onPercept ? await this.handlers.onPercept(req.params) : { accepted: true };
        break;
      case "action/execute": {
        const actionId = String(req.params.action_id);
        this.journal.set(actionId, { state: "in_flight" });
        const res = await this.handlers.execute(req.params);
        if (res.receipt) this.journal.set(actionId, { state: "terminal", receipt: res.receipt });
        result = res;
        break;
      }
      case "action/cancel": {
        const res = await this.handlers.cancel(req.params);
        if (res.receipt) this.journal.set(String(req.params.action_id), { state: "terminal", receipt: res.receipt });
        result = res;
        break;
      }
      case "action/receipt": {
        const receipt = req.params.receipt as Receipt;
        this.journal.set(receipt.action_id, { state: "terminal", receipt });
        result = { recorded: true };
        break;
      }
      case "approval/resolve": result = await this.handlers.resolveApproval(req.params); break;
      case "question/answer": result = this.handlers.onQuestion ? await this.handlers.onQuestion(req.params) : { accepted: true }; break;
      case "state/query": result = this.queryState(String(req.params.action_id)); break;
      case "blob/put": {
        const raw = Array.isArray(req.params.inline_bytes) ? Uint8Array.from(req.params.inline_bytes as number[]) : new Uint8Array();
        if (req.params.inline_bytes !== undefined && raw.length !== Number(req.params.byte_len)) {
          throw new RpcFailure(-32005, "ERR_HASH_MISMATCH: byte_len mismatch");
        }
        const digest = req.params.inline_bytes !== undefined ? verifySha256(String(req.params.sha256), raw) : String(req.params.sha256);
        const blobRef = `blob://sha256/${digest}`;
        this.blobs.set(blobRef, raw);
        result = { blob_ref: blobRef, sha256: digest, expires_at_ms: 60_000 };
        break;
      }
      case "blob/get": {
        const blobRef = String(req.params.blob_ref);
        const raw = this.blobs.get(blobRef) ?? new Uint8Array();
        const sha256 = req.params.expected_sha256 ? verifySha256(String(req.params.expected_sha256), raw) : sha256Hex(raw);
        result = { blob_ref: blobRef, sha256, byte_len: raw.length, bytes: Array.from(raw) };
        break;
      }
    }
    this.idem.set(key, result);
    return result;
  }
}
