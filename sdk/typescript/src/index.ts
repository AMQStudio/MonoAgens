import { createHash } from "node:crypto";
import { createConnection, type Socket } from "node:net";
import { createInterface, type Interface as ReadlineInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";

export const OPENRPC_SCHEMA_PATH = "../../crates/protocol/openrpc/monoagens.body.v1.openrpc.json";
export const PROTOCOL_VERSION = { major: 1, minor: 0 } as const;
export const WAIST2_METHODS = [
  "body/discover", "body/heartbeat", "body/shutdown", "percept/emit",
  "action/execute", "action/cancel", "action/receipt", "approval/resolve",
  "question/answer", "state/query", "blob/put", "blob/get",
] as const;

export type Waist2Method = (typeof WAIST2_METHODS)[number];
export type RpcId = number | string | null;
export type BodyStatus = "ready" | "busy" | "degraded" | "stopped";
export type Taint = "untrusted" | "provisional" | "trusted";
export type ModalitySlot = "image" | "audio_voice" | "video" | "sensor_blob";

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

export interface AttachmentHandle {
  handle_id: string;
  modality_slot: ModalitySlot;
  byte_len: number;
  expires_at_ms: number;
  embed_model_ver: string;
}

export interface OrientScore {
  actor: string;
  score: number;
}

export interface ChannelPrediction {
  channel: string;
  delta_obs?: number;
  delta_pred?: number;
}

export interface EmbeddingTag {
  embed_model_ver: string;
  dim_d: number;
  prompt_regime_id: string;
}

export interface TaggedEmbedding {
  tag: EmbeddingTag;
  values: number[];
}

export interface PerceptEmitInput {
  headers?: Partial<Waist2Headers>;
  actor: string;
  origin_actor?: string;
  place: string;
  audience: string[];
  timestamp_ms: number;
  payload: number[] | Uint8Array;
  taint?: Taint;
  addressees?: string[];
  named?: string[];
  orient?: OrientScore[];
  reply_to?: string;
  correlation_id?: string;
  channel?: string;
  delta_obs?: number;
  channel_prediction?: ChannelPrediction;
  length?: number;
  negative_flag?: boolean;
  repair?: boolean;
  continued_attention?: boolean;
  f_modal?: number[];
  embeddings?: TaggedEmbedding[];
  attachments?: AttachmentHandle[];
  dialogue_act_gap?: number;
  peer_expertise_margin?: number;
}

export interface PerceptEmitResult {
  accepted: boolean;
}

export interface ActionReceiptResult {
  recorded: boolean;
}

export interface BodyHeartbeatResult {
  acknowledged_at_ms: number;
}

export interface BlobPutResult {
  blob_ref: string;
  sha256: string;
  expires_at_ms: number;
}

export interface BlobGetResult {
  blob_ref: string;
  sha256: string;
  byte_len: number;
  bytes: number[];
}

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
  readonly data?: Record<string, unknown>;
  constructor(code: number, message: string, data?: Record<string, unknown>) {
    super(message);
    this.code = code;
    this.data = data;
  }
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
  nextRequest(id: RpcId, bodyId: string, status: BodyStatus, nowMs: number, key: string): Waist2Request {
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

export interface BodySessionOptions {
  defaultTimeoutMs?: number;
  idPrefix?: string;
}

interface PendingRpc {
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
  timer?: ReturnType<typeof setTimeout>;
}

function rpcIdKey(id: RpcId): string {
  if (typeof id === "number") return `n:${id}`;
  if (typeof id === "string") return `s:${id}`;
  return "null";
}

/**
 * Bidirectional multiplexed Waist-2 NDJSON session over any Readable/Writable pair
 * (stdio, Unix domain socket, or in-memory streams).
 */
export class BodySession {
  private readonly adapter: BodyAdapter;
  private readonly input: Readable;
  private readonly output: Writable;
  private readonly ownedSocket?: Socket;
  private readonly rl: ReadlineInterface;
  private readonly pending = new Map<string, PendingRpc>();
  private readonly defaultTimeoutMs?: number;
  private readonly idPrefix: string;
  private nextRpcId = 1;
  private nextIdemSeq = 1;
  private closed = false;

  constructor(
    adapter: BodyAdapter,
    input: Readable,
    output: Writable,
    options?: BodySessionOptions,
    ownedSocket?: Socket,
  ) {
    this.adapter = adapter;
    this.input = input;
    this.output = output;
    this.ownedSocket = ownedSocket;
    this.defaultTimeoutMs = options?.defaultTimeoutMs;
    this.idPrefix = options?.idPrefix ?? "ts";

    this.rl = createInterface({ input, crlfDelay: Infinity });
    this.rl.on("line", (line) => {
      void this.onLine(line);
    });
    this.rl.on("close", () => {
      this.close("body stream closed");
    });
    this.input.on("error", (err) => {
      this.close(`input stream error: ${err.message}`);
    });
    if (this.output !== (this.input as unknown as Writable)) {
      this.output.on("error", (err) => {
        this.close(`output stream error: ${err.message}`);
      });
    }
  }

  static fromStdio(adapter: BodyAdapter, options?: BodySessionOptions): BodySession {
    return new BodySession(adapter, process.stdin, process.stdout, options);
  }

  static connectUnix(
    socketPath: string,
    adapter: BodyAdapter,
    options?: BodySessionOptions,
  ): Promise<BodySession> {
    return new Promise((resolve, reject) => {
      const socket = createConnection(socketPath);
      const onError = (err: Error) => {
        socket.removeListener("connect", onConnect);
        reject(err);
      };
      const onConnect = () => {
        socket.removeListener("error", onError);
        resolve(new BodySession(adapter, socket, socket, options, socket));
      };
      socket.once("error", onError);
      socket.once("connect", onConnect);
    });
  }

  get isClosed(): boolean {
    return this.closed;
  }

  private nextIdempotencyKey(tag: string): string {
    const seq = this.nextIdemSeq++;
    return `${this.idPrefix}-${tag}-${seq}`;
  }

  private buildHeaders(tag: string, partial?: Partial<Waist2Headers>): Waist2Headers {
    const headers: Waist2Headers = {
      idempotency_key: partial?.idempotency_key ?? this.nextIdempotencyKey(tag),
      protocol_version: partial?.protocol_version ? { ...partial.protocol_version } : { ...PROTOCOL_VERSION },
    };
    if (partial?.traceparent !== undefined) headers.traceparent = partial.traceparent;
    if (partial?._meta !== undefined) headers._meta = { ...partial._meta };
    return headers;
  }

  private writeLine(line: string): void {
    if (this.closed) {
      throw new RpcFailure(-32003, "body session is closed");
    }
    this.output.write(`${line}\n`);
  }

  private async onLine(line: string): Promise<void> {
    if (!line.trim()) return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (!this.closed) {
        this.writeLine(JSON.stringify({
          jsonrpc: "2.0",
          id: null,
          error: { code: -32700, message: `parse error: ${message}` },
        } satisfies Waist2Response));
      }
      return;
    }

    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return;
    const obj = parsed as Record<string, unknown>;

    if (typeof obj.method === "string") {
      try {
        const out = await this.adapter.handleLine(line);
        if (out !== null && "id" in obj && obj.id !== undefined && !this.closed) {
          this.writeLine(out);
        }
      } catch (err) {
        if ("id" in obj && obj.id !== undefined && !this.closed) {
          const code = err instanceof RpcFailure ? err.code : -32003;
          const message = err instanceof Error ? err.message : String(err);
          this.writeLine(JSON.stringify({
            jsonrpc: "2.0",
            id: (obj.id as RpcId) ?? null,
            error: { code, message },
          } satisfies Waist2Response));
        }
      }
      return;
    }

    if ("id" in obj && obj.id !== undefined && obj.id !== null) {
      const resp = obj as unknown as Waist2Response;
      const key = rpcIdKey(resp.id);
      const entry = this.pending.get(key);
      if (!entry) return;
      this.pending.delete(key);
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      if (resp.error !== undefined && resp.error !== null) {
        entry.reject(new RpcFailure(resp.error.code, resp.error.message, resp.error.data));
      } else {
        entry.resolve(resp.result);
      }
    }
  }

  call<R = unknown>(
    method: Waist2Method,
    params: Record<string, unknown>,
    timeoutMs?: number,
  ): Promise<R> {
    if (this.closed) {
      return Promise.reject(new RpcFailure(-32003, "body session is closed"));
    }
    const id = this.nextRpcId++;
    const key = rpcIdKey(id);
    const existingHeaders = params.headers as Partial<Waist2Headers> | undefined;
    const headers = this.buildHeaders(method.replace("/", "-"), existingHeaders);
    const req: Waist2Request<Record<string, unknown>> = {
      jsonrpc: "2.0",
      id,
      method,
      params: { ...params, headers },
    };
    const wire = JSON.stringify(req);
    const effectiveTimeout = timeoutMs ?? this.defaultTimeoutMs;

    return new Promise<R>((resolve, reject) => {
      const entry: PendingRpc = {
        resolve: resolve as (value: unknown) => void,
        reject,
      };
      if (effectiveTimeout !== undefined && effectiveTimeout > 0) {
        entry.timer = setTimeout(() => {
          if (this.pending.delete(key)) {
            reject(new RpcFailure(-32003, `RPC '${method}' timed out after ${effectiveTimeout}ms`));
          }
        }, effectiveTimeout);
      }
      this.pending.set(key, entry);
      try {
        this.writeLine(wire);
      } catch (err) {
        this.pending.delete(key);
        if (entry.timer !== undefined) clearTimeout(entry.timer);
        reject(err);
      }
    });
  }

  emitPercept(params: PerceptEmitInput, timeoutMs?: number): Promise<PerceptEmitResult> {
    if (!Array.isArray(params.audience) || params.audience.length === 0) {
      return Promise.reject(
        new RpcFailure(-32006, "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience"),
      );
    }
    const payload = params.payload instanceof Uint8Array
      ? Array.from(params.payload)
      : params.payload;
    const wireParams: Record<string, unknown> = {
      ...params,
      payload,
      taint: params.taint ?? "untrusted",
    };
    return this.call<PerceptEmitResult>("percept/emit", wireParams, timeoutMs);
  }

  sendReceipt(
    receipt: Receipt,
    idempotencyKey?: string,
    timeoutMs?: number,
  ): Promise<ActionReceiptResult> {
    const headers = this.buildHeaders(
      "receipt",
      idempotencyKey !== undefined ? { idempotency_key: idempotencyKey } : undefined,
    );
    return this.call<ActionReceiptResult>("action/receipt", { headers, receipt }, timeoutMs);
  }

  sendHeartbeat(
    bodyId: string,
    status: BodyStatus,
    timestampMs = Date.now(),
    timeoutMs?: number,
  ): Promise<BodyHeartbeatResult> {
    return this.call<BodyHeartbeatResult>(
      "body/heartbeat",
      { body_id: bodyId, status, timestamp_ms: timestampMs },
      timeoutMs,
    );
  }

  putBlob(
    bytes: Uint8Array,
    contentType?: string,
    scopeId?: string,
    timeoutMs?: number,
  ): Promise<BlobPutResult> {
    const sha256 = sha256Hex(bytes);
    const meta: Record<string, unknown> = {};
    if (contentType !== undefined) meta.content_type = contentType;
    if (scopeId !== undefined) meta.scope_id = scopeId;
    const headers = this.buildHeaders(
      "blob-put",
      Object.keys(meta).length > 0 ? { _meta: meta } : undefined,
    );
    return this.call<BlobPutResult>(
      "blob/put",
      {
        headers,
        sha256,
        byte_len: bytes.byteLength,
        inline_bytes: Array.from(bytes),
      },
      timeoutMs,
    );
  }

  async getBlob(
    blobRef: string,
    expectedSha256?: string,
    timeoutMs?: number,
  ): Promise<BlobGetResult> {
    const params: Record<string, unknown> = { blob_ref: blobRef };
    if (expectedSha256 !== undefined) params.expected_sha256 = expectedSha256;
    const res = await this.call<{
      blob_ref: string;
      sha256: string;
      byte_len: number;
      bytes?: number[];
    }>("blob/get", params, timeoutMs);

    const rawBytes = res.bytes ?? [];
    if (expectedSha256 !== undefined && res.sha256.toLowerCase() !== expectedSha256.toLowerCase()) {
      throw new RpcFailure(
        -32005,
        `ERR_HASH_MISMATCH: expected ${expectedSha256}, got ${res.sha256}`,
      );
    }
    verifySha256(expectedSha256 ?? res.sha256, Uint8Array.from(rawBytes));
    return {
      blob_ref: res.blob_ref,
      sha256: res.sha256,
      byte_len: res.byte_len,
      bytes: rawBytes,
    };
  }

  close(reason = "body session closed"): void {
    if (this.closed) return;
    this.closed = true;
    this.rl.close();
    for (const [key, entry] of this.pending.entries()) {
      this.pending.delete(key);
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      entry.reject(new RpcFailure(-32003, reason));
    }
    if (this.ownedSocket !== undefined) {
      this.ownedSocket.destroy();
    }
  }
}
