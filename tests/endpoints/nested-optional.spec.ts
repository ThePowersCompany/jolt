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
// The nested `filter` struct is entirely optional (no required keys),
// so it does not form a gated group.
// Any subset of its keys, including none, is valid.

describe(endpoint, () => {
  it("accepts an empty request (all defaults)", async () => {
    const res = await GET(endpoint);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
  });

  it("accepts just the base page key", async () => {
    const res = await GET(endpoint, { queryParams: { page: 2 } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.page, 2);
    assert.strictEqual(body.category, null);
    assert.strictEqual(body.status, null);
  });

  it("accepts only category from the nested struct", async () => {
    const res = await GET(endpoint, { queryParams: { category: "books" } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    // `category` present while `status` stays null proves there is no all-or-nothing gating.
    assert.strictEqual(body.category, "books");
    assert.strictEqual(body.status, null);
    // `page` falls back to its default of 1.
    assert.strictEqual(body.page, 1);
  });

  it("accepts only status from the nested struct", async () => {
    const res = await GET(endpoint, { queryParams: { status: "active" } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.status, "active");
    assert.strictEqual(body.category, null);
  });

  it("accepts both nested keys together", async () => {
    const res = await GET(endpoint, {
      queryParams: { category: "books", status: "active" },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.category, "books");
    assert.strictEqual(body.status, "active");
  });

  it("accepts page combined with a partial filter", async () => {
    const res = await GET(endpoint, {
      queryParams: { page: 3, status: "active" },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.page, 3);
    assert.strictEqual(body.status, "active");
    assert.strictEqual(body.category, null);
  });

  it("rejects a page of the wrong type", async () => {
    const res = await GET(endpoint, "?page=abc");
    assert.strictEqual(res.status, 400);
    assert.strictEqual("Incorrect query parameter for: page", await res.text());
  });
});
