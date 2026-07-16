import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/gated-group";

// Testing:
// query_params: struct {
//     room: i32,
//     cursor: Optional(struct {
//         cursor: i64,
//         before: bool = false,
//     }) = .not_provided,
//     limit: u32 = 10,
// }
//
// The `cursor` nested struct is a gated group: if any of its keys are present,
// the required `cursor` key must be present too.

describe(endpoint, () => {
  describe("base keys", () => {
    it("accepts the required room with no gated group", async () => {
      const res = await GET(`${endpoint}?room=1`, null);
      assert.strictEqual(res.status, 204, await res.text());
    });

    it("accepts room together with the defaulted limit", async () => {
      const res = await GET(`${endpoint}?room=1&limit=5`, null);
      assert.strictEqual(res.status, 204, await res.text());
    });

    it("rejects a missing required room", async () => {
      const res = await GET(endpoint, null);
      assert.strictEqual(res.status, 400);
    });

    it("rejects a room of the wrong type", async () => {
      const res = await GET(`${endpoint}?room=abc`, null);
      assert.strictEqual(res.status, 400);
    });
  });

  describe("gated group", () => {
    it("accepts the group present with only its required key", async () => {
      const res = await GET(`${endpoint}?room=1&cursor=100`, null);
      assert.strictEqual(res.status, 204, await res.text());
    });

    it("accepts the group present with its required and optional keys", async () => {
      const res = await GET(`${endpoint}?room=1&cursor=100&before=true`, null);
      assert.strictEqual(res.status, 204, await res.text());
    });

    it("rejects partial presence: optional key without the required key", async () => {
      const res = await GET(`${endpoint}?room=1&before=true`, null);
      assert.strictEqual(res.status, 400);
    });

    it("rejects the group's required key of the wrong type", async () => {
      const res = await GET(`${endpoint}?room=1&cursor=notanumber`, null);
      assert.strictEqual(res.status, 400);
    });
  });
});

