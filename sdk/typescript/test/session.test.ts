import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:net";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { PassThrough } from "node:stream";
import {
  BodyAdapter,
  BodySession,
  RpcFailure,
  sha256Hex,
  type Waist2Request,
  type Waist2Response,
} from "../src/index.ts";

function makeAdapter() {
  return new BodyAdapter({
    discover: () => ({
      body_id: "body-ts-mux",
      protocol_version: { major: 1, minor: 0 },
      capabilities: [],
      places: ["wa:chat-1"],
    }),
    execute: (p) => ({
      accepted: true,
      receipt: {
        action_id: String(p.action_id),
        capability_id: String(p.capability_id),
        outcome: "done",
        termination_reason: "normal",
        timestamp_ms: 1234,
      },
    }),
    cancel: (p) => ({
      cancelled: true,
      receipt: {
        action_id: String(p.action_id),
        capability_id: "chat.send",
        outcome: "failed",
        timestamp_ms: 1235,
      },
    }),
    resolveApproval: () => ({ applied: true }),
  });
}

test("BodySession: bidirectional multiplexing interleaves outbound RPCs and inbound requests", async () => {
  const gwToBody = new PassThrough();
  const bodyToGw = new PassThrough();
  const session = new BodySession(makeAdapter(), gwToBody, bodyToGw);

  const gwSeenRequests: Waist2Request[] = [];
  const gwResponses = new Map<number, Waist2Response>();
  const blobs = new Map<string, number[]>();

  const gwRl = createInterface({ input: bodyToGw, crlfDelay: Infinity });
  gwRl.on("line", (line) => {
    const msg = JSON.parse(line) as Record<string, unknown>;
    if (typeof msg.method === "string") {
      const req = msg as unknown as Waist2Request<Record<string, unknown>>;
      gwSeenRequests.push(req);
      // Respond out-of-order / asynchronously to test correlation by id.
      const delay = req.method === "percept/emit" ? 15 : 2;
      setTimeout(() => {
        if (req.method === "percept/emit") {
          gwToBody.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, result: { accepted: true } }) + "\n");
        } else if (req.method === "action/receipt") {
          gwToBody.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, result: { recorded: true } }) + "\n");
        } else if (req.method === "body/heartbeat") {
          gwToBody.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, result: { acknowledged_at_ms: 9999 } }) + "\n");
        } else if (req.method === "blob/put") {
          const digest = String(req.params.sha256);
          const ref = `blob://staged/${digest}`;
          blobs.set(ref, req.params.inline_bytes as number[]);
          gwToBody.write(JSON.stringify({
            jsonrpc: "2.0",
            id: req.id,
            result: { blob_ref: ref, sha256: digest, expires_at_ms: 60_000 },
          }) + "\n");
        } else if (req.method === "blob/get") {
          const ref = String(req.params.blob_ref);
          const bytes = blobs.get(ref) ?? [];
          gwToBody.write(JSON.stringify({
            jsonrpc: "2.0",
            id: req.id,
            result: { blob_ref: ref, sha256: sha256Hex(Uint8Array.from(bytes)), byte_len: bytes.length, bytes },
          }) + "\n");
        }
      }, delay);
    } else {
      const resp = msg as unknown as Waist2Response;
      gwResponses.set(Number(resp.id), resp);
    }
  });

  // Fire outbound percept + receipt + heartbeat concurrently with inbound discover + execute!
  const pPercept = session.emitPercept({
    actor: "wa:+15550001",
    place: "wa:chat-1",
    audience: ["wa:+15550001", "owner"],
    timestamp_ms: 5000,
    payload: Uint8Array.from([104, 101, 108, 108, 111]),
  });

  const pReceipt = session.sendReceipt({
    action_id: "act-out-1",
    capability_id: "chat.send",
    outcome: "done",
    timestamp_ms: 5001,
  });

  const pHeartbeat = session.sendHeartbeat("body-ts-mux", "ready", 5002);

  gwToBody.write(JSON.stringify({
    jsonrpc: "2.0",
    id: 101,
    method: "body/discover",
    params: { headers: { idempotency_key: "gw-disc-1", protocol_version: { major: 1, minor: 0 } } },
  }) + "\n");

  gwToBody.write(JSON.stringify({
    jsonrpc: "2.0",
    id: 102,
    method: "action/execute",
    params: {
      headers: { idempotency_key: "gw-exec-1", protocol_version: { major: 1, minor: 0 } },
      action_id: "act-in-1",
      capability_id: "chat.send",
      places: ["wa:chat-1"],
      audience: ["wa:+15550001"],
      payload: [111, 107],
    },
  }) + "\n");

  const [perceptRes, receiptRes, hbRes] = await Promise.all([pPercept, pReceipt, pHeartbeat]);
  assert.deepEqual(perceptRes, { accepted: true });
  assert.deepEqual(receiptRes, { recorded: true });
  assert.deepEqual(hbRes, { acknowledged_at_ms: 9999 });

  // Wait briefly for inbound responses to arrive at mock gateway.
  await new Promise((r) => setTimeout(r, 20));
  assert.equal((gwResponses.get(101)?.result as { body_id: string }).body_id, "body-ts-mux");
  assert.equal((gwResponses.get(102)?.result as { accepted: boolean }).accepted, true);

  // Verify wire fields on percept/emit match PerceptEmitParams.
  const emitted = gwSeenRequests.find((r) => r.method === "percept/emit")!;
  assert.equal(emitted.params.actor, "wa:+15550001");
  assert.equal(emitted.params.place, "wa:chat-1");
  assert.deepEqual(emitted.params.audience, ["wa:+15550001", "owner"]);
  assert.deepEqual(emitted.params.payload, [104, 101, 108, 108, 111]);
  assert.equal(emitted.params.taint, "untrusted");
  assert.ok((emitted.params.headers as { idempotency_key: string }).idempotency_key);

  // Test putBlob + getBlob + SHA-256 verification.
  const putRes = await session.putBlob(Uint8Array.from([1, 2, 3, 4]), "image/png", "scope-a");
  assert.equal(putRes.sha256, sha256Hex(Uint8Array.from([1, 2, 3, 4])));
  const putReq = gwSeenRequests.find((r) => r.method === "blob/put")!;
  assert.equal(
    ((putReq.params.headers as { _meta?: Record<string, unknown> })._meta ?? {}).scope_id,
    "scope-a",
  );
  assert.equal(
    ((putReq.params.headers as { _meta?: Record<string, unknown> })._meta ?? {}).content_type,
    "image/png",
  );

  const got = await session.getBlob(putRes.blob_ref, putRes.sha256);
  assert.deepEqual(got.bytes, [1, 2, 3, 4]);
  await assert.rejects(
    () => session.getBlob(putRes.blob_ref, "f".repeat(64)),
    (err: unknown) => err instanceof RpcFailure && err.code === -32005,
  );

  gwRl.close();
  session.close();
});

test("BodySession: local empty-audience validation, JSON-RPC error rejection, and EOF cleanup", async () => {
  const gwToBody = new PassThrough();
  const bodyToGw = new PassThrough();
  const session = new BodySession(makeAdapter(), gwToBody, bodyToGw);

  // Empty audience fails locally without writing to stream.
  await assert.rejects(
    () =>
      session.emitPercept({
        actor: "wa:+1",
        place: "wa:chat-1",
        audience: [],
        timestamp_ms: 1,
        payload: [1],
      }),
    (err: unknown) => err instanceof RpcFailure && err.code === -32006,
  );

  const gwRl = createInterface({ input: bodyToGw, crlfDelay: Infinity });
  gwRl.on("line", (line) => {
    const req = JSON.parse(line) as Waist2Request<Record<string, unknown>>;
    if (req.method === "body/heartbeat") {
      gwToBody.write(JSON.stringify({
        jsonrpc: "2.0",
        id: req.id,
        error: { code: -32004, message: "ERR_UNSUPPORTED_VERSION: bad version" },
      }) + "\n");
    }
    // Intentionally leave action/receipt unanswered to verify EOF cleanup.
  });

  await assert.rejects(
    () => session.sendHeartbeat("body-ts", "ready", 10),
    (err: unknown) => err instanceof RpcFailure && err.code === -32004,
  );

  const pendingOnEof = session.sendReceipt({
    action_id: "act-eof",
    capability_id: "chat.send",
    outcome: "done",
    timestamp_ms: 20,
  });

  // Closing the gateway->body stream triggers EOF and cleanly rejects pending RPCs.
  gwToBody.end();
  await assert.rejects(
    () => pendingOnEof,
    (err: unknown) => err instanceof RpcFailure && err.code === -32003,
  );
  assert.equal(session.isClosed, true);
  gwRl.close();
});

test("BodySession: connectUnix works over Unix domain socket", async () => {
  const dir = mkdtempSync(join(tmpdir(), "monoagens-ts-uds-"));
  const sockPath = join(dir, "body.sock");
  const server = createServer((socket) => {
    const rl = createInterface({ input: socket, crlfDelay: Infinity });
    rl.on("line", (line) => {
      const msg = JSON.parse(line) as Waist2Request;
      if (msg.method === "percept/emit") {
        socket.write(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { accepted: true } }) + "\n");
      }
    });
  });

  await new Promise<void>((resolve) => server.listen(sockPath, resolve));
  try {
    const session = await BodySession.connectUnix(sockPath, makeAdapter());
    const res = await session.emitPercept({
      actor: "tg:42",
      place: "tg:room-1",
      audience: ["tg:42"],
      timestamp_ms: 123,
      payload: [65],
    });
    assert.deepEqual(res, { accepted: true });
    session.close();
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
