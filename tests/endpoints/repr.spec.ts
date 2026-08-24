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

    it("rejects a fractional value for an integer _repr", async () => {
      const res = await POST(endpoint, {
        body: {
          epoch_millis: 1787542979973.5,
        } as unknown as { epoch_millis: number },
      });

      assert.strictEqual(res.status, 400);
    });
  });

  describe("PATCH", () => {
    it("round-trips a float _repr inside the body object", async () => {
      const body = {
        foo: 14.7,
      };

      const res = await PATCH(endpoint, { body });
      if (res.status !== 200) {
        assert.fail(await res.text());
      }

      assert.deepStrictEqual(await res.json(), body);
    });

    it("round-trips an integer through the alternate union variant", async () => {
      const body = {
        foo: 14,
      };

      const res = await PATCH(endpoint, { body });
      if (res.status !== 200) {
        assert.fail(await res.text());
      }

      assert.deepStrictEqual(await res.json(), body);
    });

    it("round-trips negative integer and floating-point values", async () => {
      for (const foo of [-14, -14.7]) {
        const body = { foo };
        const res = await PATCH(endpoint, { body });
        if (res.status !== 200) {
          assert.fail(await res.text());
        }

        assert.deepStrictEqual(await res.json(), body);
      }
    });

    it("rejects values that do not match the scalar _repr", async () => {
      for (const foo of [true, "foobar", null, [], {}]) {
        const res = await PATCH(endpoint, {
          body: { foo } as unknown as { foo: number },
        });

        assert.strictEqual(
          res.status,
          400,
          `expected ${JSON.stringify(foo)} to fail`,
        );
      }
    });
  });
});
