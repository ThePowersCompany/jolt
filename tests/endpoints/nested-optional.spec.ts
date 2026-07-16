import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/nested-optional";

// Testing the following:
// query_params: struct {
//      page: u32 = 1,
//      filter: Optional(struct {
//          category: Optional([]const u8) = .not_provided,
//          status: Optional([]const u8) = .not_provided,
//      }) = .not_provided,
//  }
//
// The nested `filter` struct is entirely optional (no required keys), so it does not form a gated group.
// Any subset of its keys, including none, is valid.

describe(endpoint, () => {
  it("accepts an empty request (all defaults)", async () => {
    const res = await GET(endpoint, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("accepts just the base page key", async () => {
    const res = await GET(`${endpoint}?page=2`, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("accepts only category from the nested struct", async () => {
    const res = await GET(`${endpoint}?category=books`, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("accepts only status from the nested struct", async () => {
    const res = await GET(`${endpoint}?status=active`, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("accepts both nested keys together", async () => {
    const res = await GET(`${endpoint}?category=books&status=active`, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("accepts page combined with a partial filter", async () => {
    const res = await GET(`${endpoint}?page=3&status=active`, null);
    assert.strictEqual(res.status, 204, await res.text());
  });

  it("rejects a page of the wrong type", async () => {
    const res = await GET(`${endpoint}?page=abc`, null);
    assert.strictEqual(res.status, 400);
  });
});

