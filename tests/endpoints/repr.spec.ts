import assert from "node:assert";
import { describe, it } from "node:test";
import { PATCH, POST } from "../utils/client";

const endpoint = "/repr";

// Testing:
//
// const EpochMillis = struct {
//     value: DateTime,
//     pub const _repr: type = i64;
// };

describe(endpoint, () => {
  describe("POST", () => {
    it("uses _repr wire types for both the request and response", async () => {
      const body = {
        epoch_millis: 1787542979973,
      };

      const res = await POST(endpoint, { body });
      if (res.status !== 200) {
        assert.fail(await res.text());
      }

      assert.deepStrictEqual(await res.json(), body);
    });

    it("rejects a body that does not match the numeric _repr", async () => {
      const res = await POST(endpoint, {
        body: {
          epoch_millis: "foobar",
        } as unknown as { epoch_millis: number },
      });

      assert.strictEqual(res.status, 400);
    });
  });

  describe("PATCH", () => {
    it("float", async () => {
      const body = {
        foo: 14.7,
      };

      const res = await PATCH(endpoint, { body });
      if (res.status !== 200) {
        assert.fail(await res.text());
      }

      assert.deepStrictEqual(await res.json(), body);
    });

    it("integer", async () => {
      const body = {
        foo: 14.0,
      };

      const res = await PATCH(endpoint, { body });
      if (res.status !== 200) {
        assert.fail(await res.text());
      }

      assert.deepStrictEqual(await res.json(), body);
    });

    it("rejects a body that does not match the _repr type", async () => {
      const res = await PATCH(endpoint, {
        body: {
          foo: true,
        } as unknown as { foo: number },
      });

      assert.strictEqual(res.status, 400);
    });
  });
});
