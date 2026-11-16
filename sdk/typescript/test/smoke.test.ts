import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { BodyAdapter, HeartbeatScheduler, sha256Hex, type Waist2Request } from "../src/index.ts";

test("smoke: round-trip against schema, dedup, state/query, sha256 blob, heartbeat", async () => {
  const schema = JSON.parse(
    readFileSync(new URL("../../../crates/protocol/jsonschema/Waist2Request.schema.json", import.meta.url), "utf8"),
  ) as {
    required: string[];
    properties: { jsonrpc: { const: string } };
    $defs: { Waist2Method: { enum: string[] } };
  };

  const req: Waist2Request = {
    jsonrpc: "2.0",
    id: 1,
    method: "action/execute",
    params: {
      headers: { idempotency_key: "idem-ts-1", protocol_version: { major: 1, minor: 0 } },
      action_id: "act-ts-1",
      capability_id: "chat.speak",
      places: ["cli:0"],
      audience: ["per_alice"],
      payload: [104, 105],
    },
  };
  const wire = JSON.stringify(req);
  const parsed = JSON.parse(wire) as Waist2Request;
  assert.deepEqual(parsed, req);
  for (const field of schema.required) assert.ok(field in parsed);
  assert.equal(parsed.jsonrpc, schema.properties.jsonrpc.const);
  assert.ok(schema.$defs.Waist2Method.enum.includes(parsed.method));

  let execCount = 0;
  const adapter = new BodyAdapter({
    discover: () => ({ body_id: "body-ts", protocol_version: { major: 1, minor: 0 }, capabilities: [], places: ["cli:0"] }),
    execute: (p) => {
      execCount += 1;
      return {
        accepted: true,
        receipt: {
          action_id: String(p.action_id), capability_id: String(p.capability_id),
          outcome: "done", termination_reason: "normal", progress: 1.0, delivered_units: 1, timestamp_ms: 100,
        },
      };
    },
    cancel: (p) => ({ cancelled: true, receipt: { action_id: String(p.action_id), capability_id: "chat.speak", outcome: "failed", timestamp_ms: 101 } }),
    resolveApproval: () => ({ applied: true }),
  });

  for (let i = 0; i < 3; i++) {
    const res = JSON.parse((await adapter.handleLine(wire))!) as { result: { accepted: boolean } };
    assert.equal(res.result.accepted, true);
  }
  assert.equal(execCount, 1);

  const qResp = JSON.parse((await adapter.handleLine(JSON.stringify({
    jsonrpc: "2.0", id: 2, method: "state/query",
    params: { headers: { idempotency_key: "idem-q", protocol_version: { major: 1, minor: 0 } }, action_id: "act-ts-1" },
  })))!) as { result: { status: { state: string } } };
  assert.equal(qResp.result.status.state, "terminal");
  assert.equal(adapter.queryState("missing").status.state, "unknown");

  const badBlob = JSON.parse((await adapter.handleLine(JSON.stringify({
    jsonrpc: "2.0", id: 3, method: "blob/put",
    params: { headers: { idempotency_key: "idem-b", protocol_version: { major: 1, minor: 0 } }, sha256: "0".repeat(64), byte_len: 3, inline_bytes: [97, 98, 99] },
  })))!) as { error: { code: number } };
  assert.equal(badBlob.error.code, -32005);
  assert.equal(sha256Hex(Uint8Array.from([97, 98, 99])), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

  const hb = new HeartbeatScheduler(1_000);
  assert.equal(hb.isDue(100), true);
  hb.nextRequest(4, "body-ts", "ready", 100, "idem-hb");
  assert.equal(hb.isDue(500), false);
  assert.equal(hb.isDue(1_100), true);
});
