import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/any-of";

// Testing:
// query_params: struct {
//     query: Optional([]const u8) = .not_provided,
//     email: Optional([]const u8) = .not_provided,
//     username: Optional([]const u8) = .not_provided,
//     role: Optional([]const u8) = .not_provided,
//
//     pub const constraints: Constraints = .{ .any_of = true };
// }
//
// The query_params are all optional (query, email, username, role)
// but carry an `any_of` constraint,
// so at least one of them must be present.

describe(endpoint, () => {
  it("rejects a request with none of the fields", async () => {
    const res = await GET(endpoint, null);
    assert.strictEqual(res.status, 400);
  });

  it("accepts a request with just query", async () => {
    const res = await GET(`${endpoint}?query=hello`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.query, "hello");
    // The other optional fields remain null
    assert.strictEqual(body.email, null);
    assert.strictEqual(body.username, null);
    assert.strictEqual(body.role, null);
  });

  it("accepts a request with just email", async () => {
    const res = await GET(`${endpoint}?email=a@b.com`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.email, "a@b.com");
    assert.strictEqual(body.query, null);
  });

  it("accepts a request with just username", async () => {
    const res = await GET(`${endpoint}?username=alice`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.username, "alice");
  });

  it("accepts a request with just role", async () => {
    const res = await GET(`${endpoint}?role=admin`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.role, "admin");
  });

  it("accepts a request with several fields", async () => {
    const res = await GET(`${endpoint}?query=hello&role=admin`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.query, "hello");
    assert.strictEqual(body.role, "admin");
    assert.strictEqual(body.email, null);
    assert.strictEqual(body.username, null);
  });

  it("rejects when only an unknown field is provided", async () => {
    const res = await GET(`${endpoint}?unknown=1`, null);
    assert.strictEqual(res.status, 400);
  });
});
