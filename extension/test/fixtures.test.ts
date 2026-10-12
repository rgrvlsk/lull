// Decodes every golden exchange in ../../protocol/fixtures. In M0 only the JSON-RPC
// envelope is checked; the typed decode against the extension's own protocol types is
// skipped until WP2.2 provides them.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const fixturesDir = join(dirname(fileURLToPath(import.meta.url)), "../../protocol/fixtures");
const files = readdirSync(fixturesDir).filter((f) => f.endsWith(".ndjson")).sort();

type Envelope = { jsonrpc: "2.0"; id?: number | string; method?: string; params?: unknown; result?: unknown; error?: { code: number; message: string } };

test("fixture set is present", () => {
  assert.ok(files.length >= 22, `found ${files.length} fixtures`);
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

  test(`${file} decodes with the extension's typed protocol`, { skip: "until WP2.2" }, () => {});
}
