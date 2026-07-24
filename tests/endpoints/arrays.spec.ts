import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/arrays";

// Testing:
// const Tag = enum { red, green, blue };
//
// const GetContext = struct {
//     query_params: struct {
//         ids: Optional([]const u32) = .not_provided,
//         tags: Optional([]const Tag) = .not_provided,
//     },
// };

describe(endpoint, () => {
  it("parses a comma separated int array", async () => {
    const res = await GET(endpoint, { queryParams: { ids: [1, 2, 3] } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.deepStrictEqual(body.ids, [1, 2, 3]);
    assert.deepStrictEqual(body.tags, []);
  });

  it("parses a single element int array", async () => {
    const res = await GET(endpoint, { queryParams: { ids: [32] } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.deepStrictEqual(body.ids, [32]);
  });

  it("parses a comma separated enum array", async () => {
    const res = await GET(endpoint, {
      queryParams: { tags: ["red", "blue"] },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.deepStrictEqual(body.tags, ["red", "blue"]);
  });

  it("parses both arrays together", async () => {
    const res = await GET(endpoint, {
      queryParams: { ids: [7, 8], tags: ["green"] },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.deepStrictEqual(body.ids, [7, 8]);
    assert.deepStrictEqual(body.tags, ["green"]);
  });

  it("accepts a request with no arrays at all", async () => {
    const res = await GET(endpoint);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.deepStrictEqual(body.ids, []);
    assert.deepStrictEqual(body.tags, []);
  });

  it("rejects a non-numeric element in an int array", async () => {
    const res = await GET(endpoint, "?ids=1,x,3");
    assert.strictEqual(res.status, 400);
    assert.strictEqual("Incorrect query parameter for: ids", await res.text());
  });

  it("rejects an unknown enum variant", async () => {
    const res = await GET(endpoint, "?tags=red,purple");
    assert.strictEqual(res.status, 400);
  });
});
