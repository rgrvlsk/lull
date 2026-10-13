// Every golden line must pass the extension's wire guards, including CLI/app methods.
import { test } from "node:test";
import { decodeMessage } from "../src/protocol.ts";
import type { Method } from "../src/protocol.ts";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const fixturesDir = join(dirname(fileURLToPath(import.meta.url)), "../../protocol/fixtures");
const files = readdirSync(fixturesDir).filter((f) => f.endsWith(".ndjson")).sort();

type Envelope = { jsonrpc: "2.0"; id?: number | string; method?: string; params?: unknown; result?: unknown; error?: { code: number; message: string } };

test("fixture set is present", () => {
  assert.ok(files.length >= 23, `found ${files.length} fixtures`);
});

for (const file of files) {
  test(`${file} is a valid JSON-RPC exchange`, () => {
    const lines = readFileSync(join(fixturesDir, file), "utf8").trimEnd().split("\n");
    const pending = new Map<number | string, string>();
    for (const [i, line] of lines.entries()) {
      const msg = JSON.parse(line) as Envelope;
      const where = `${file}:${i + 1}`;
      assert.equal(msg.jsonrpc, "2.0", where);
      if (msg.method !== undefined && msg.id !== undefined) {
        pending.set(msg.id, msg.method);
      } else if (msg.method !== undefined) {
        assert.match(msg.method, /^server\.(added|updated|removed)$/, where);
      } else {
        assert.ok(msg.id !== undefined, where);
        assert.ok(pending.delete(msg.id), `${where}: response to unknown id`);
        assert.ok((msg.result === undefined) !== (msg.error === undefined), where);
      }
    }
    assert.equal(pending.size, 0, `${file}: unanswered requests`);
  });

  test(`${file} decodes with the extension's typed protocol`, () => {
    const pending = new Map<number | string, Method>();
    for (const line of readFileSync(join(fixturesDir, file), "utf8").trimEnd().split("\n")) {
      const raw: unknown = JSON.parse(line);
      const envelope = raw as Envelope;
      const message = decodeMessage(raw, envelope.id === undefined ? undefined : pending.get(envelope.id));
      assert.deepEqual(message, raw);
      if ("method" in message && "id" in message) pending.set(message.id, message.method);
      else if ("id" in message) pending.delete(message.id);
    }
    assert.equal(pending.size, 0);
  });
}

test("wire guards reject malformed payloads and unknown enum values", () => {
  const request = { jsonrpc: "2.0", id: "d1", method: "tab.open", params: { url: "http://localhost:5187", focus: false } };
  for (const params of [{ url: 42, focus: true }, { url: "http://localhost", focus: "false" }]) {
    assert.throws(() => decodeMessage({ ...request, params }));
  }
  assert.throws(() => decodeMessage({ jsonrpc: "2.0", method: "server.removed", params: { seq: 1, id: "s", reason: "bored" } }));
  assert.throws(() => decodeMessage({ ...request, id: null }));
  assert.throws(() => decodeMessage({ jsonrpc: "2.0", id: 1, result: { ok: true }, error: { code: 1, message: "no" } }, "tab.open"));
  assert.throws(() => decodeMessage({ jsonrpc: "2.0", method: "server.removed", params: { seq: -1, id: "s", reason: "idle" } }));
  assert.throws(() => decodeMessage({ jsonrpc: "2.0", id: 1, result: { seq: 1, servers: [{ id: "only-an-id" }] } }, "subscribe"));
});

test("wire guards ignore unknown fields (additive minors, contract §8)", () => {
  const message = { jsonrpc: "2.0", id: "d1", method: "tab.open", params: { url: "http://localhost:5187", focus: false, extra: true } };
  assert.deepEqual(decodeMessage(message), message);
});

test("browser.tabs.sync accepts an explicit null groupId", () => {
  const line = '{"jsonrpc":"2.0","id":1,"method":"browser.tabs.sync","params":{"tabs":[{"tabId":812,"windowId":3,"url":"http://localhost:5187/","groupId":null,"active":false,"windowFocused":true}]}}';
  const message: unknown = JSON.parse(line);
  assert.deepEqual(decodeMessage(message), message);
});
