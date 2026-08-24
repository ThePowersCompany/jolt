import assert from "node:assert";
import { describe, it } from "node:test";
import { POST } from "../utils/client";

const endpoint = "/repr";

// Testing:
//
// const EpochMillis = struct {
//     value: DateTime,
//     pub const _repr: type = i64;
// };

describe(endpoint, () => {
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
