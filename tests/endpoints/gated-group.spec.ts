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
      const res = await GET(endpoint, { queryParams: { room: 1 } });
      if (res.status != 200) {
        assert.fail(await res.text());
      }

      const body = await res.json();
      assert.strictEqual(body.room, 1);

      // The gated group is absent as a unit
      assert.strictEqual(body.cursor_present, false);
      assert.strictEqual(body.cursor, null);

      // `limit` falls back to its default of 10
      assert.strictEqual(body.limit, 10);
    });

    it("accepts room together with a default field (limit)", async () => {
      const res = await GET(endpoint, {
        queryParams: { room: 1, limit: 5 },
      });
      if (res.status != 200) {
        assert.fail(await res.text());
      }
      const body = await res.json();
      assert.strictEqual(body.room, 1);
      assert.strictEqual(body.limit, 5);
    });

    it("rejects a missing required room", async () => {
      const res = await GET(endpoint);
      assert.strictEqual(res.status, 400);
      assert.strictEqual("No query params were provided", await res.text());
    });

    it("rejects a room of the wrong type", async () => {
      const res = await GET(endpoint, "?room=abc");
      assert.strictEqual(res.status, 400);
      assert.strictEqual(
        "Incorrect query parameter for: room",
        await res.text(),
      );
    });
  });

  describe("gated group", () => {
    it("accepts the group present with only its required key", async () => {
      const res = await GET(endpoint, {
        queryParams: { room: 1, cursor: 100 },
      });
      if (res.status != 200) {
        assert.fail(await res.text());
      }
      const body = await res.json();
      assert.strictEqual(body.cursor_present, true);
      assert.strictEqual(body.cursor, 100);
      // `before` defaults to false when the group is present but the key is omitted.
      assert.strictEqual(body.before, false);
    });

    it("accepts the group present with its required and optional keys", async () => {
      const res = await GET(endpoint, {
        queryParams: { room: 1, cursor: 100, before: true },
      });
      if (res.status != 200) {
        assert.fail(await res.text());
      }
      const body = await res.json();
      assert.strictEqual(body.cursor_present, true);
      assert.strictEqual(body.cursor, 100);
      assert.strictEqual(body.before, true);
    });

    it("rejects partial presence", async () => {
      const res = await GET(endpoint, "?room=1&before=true");
      assert.strictEqual(res.status, 400);
      assert.strictEqual("Missing query parameter: cursor", await res.text());
    });

    it("rejects the group's required key of the wrong type", async () => {
      const res = await GET(endpoint, "?room=1&cursor=notanumber");
      assert.strictEqual(res.status, 400);
      assert.strictEqual(
        "Incorrect query parameter for: cursor",
        await res.text(),
      );
    });
  });
});
