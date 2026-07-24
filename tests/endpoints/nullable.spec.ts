import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/nullable";

// Testing:
// query_params: struct {
//     first: ?i32,
//     second: ?i32,
// }

describe(endpoint, () => {
  it("rejects the request when a required field is missing", async () => {
    const res = await GET(endpoint, { queryParams: { first: 1 } });
    assert.strictEqual(res.status, 400);
  });

  it("accepts both required fields with null values", async () => {
    const res = await GET(endpoint, {
      queryParams: { first: null, second: null },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.first_is_null, true);
    assert.strictEqual(body.first_value, null);
    assert.strictEqual(body.second_is_null, true);
    assert.strictEqual(body.second_value, null);
  });

  it("accepts both required fields with concrete values", async () => {
    const res = await GET(endpoint, {
      queryParams: { first: 32, second: 64 },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.first_is_null, false);
    assert.strictEqual(body.first_value, 32);
    assert.strictEqual(body.second_is_null, false);
    assert.strictEqual(body.second_value, 64);
  });

  it("rejects a non-numeric first value", async () => {
    const res = await GET(endpoint, "?first=abc&second=1");
    assert.strictEqual(res.status, 400);
  });

  it("rejects a non-numeric second value", async () => {
    const res = await GET(endpoint, "?first=1&second=abc");
    assert.strictEqual(res.status, 400);
  });
});
