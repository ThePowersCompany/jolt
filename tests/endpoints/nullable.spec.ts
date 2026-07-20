import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/nullable";

// Testing:
// query_params: struct {
//     required: ?i32,
// }

describe(endpoint, () => {
  it("rejects the request when the nullable field is absent", async () => {
    const res = await GET(endpoint, null);
    assert.strictEqual(res.status, 400);
  });

  it("accepts an explicit null value", async () => {
    const res = await GET(`${endpoint}?required=null`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.is_null, true);
    assert.strictEqual(body.value, null);
  });

  it("accepts a concrete value", async () => {
    const res = await GET(`${endpoint}?required=32`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.is_null, false);
    assert.strictEqual(body.value, 32);
  });

  it("rejects a non-numeric value", async () => {
    const res = await GET(`${endpoint}?required=abc`, null);
    assert.strictEqual(res.status, 400);
  });
});
