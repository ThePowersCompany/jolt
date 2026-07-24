import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/union-variant";

// Testing:
// query_params: union(enum) {
//     id: i32,
//     date_range: struct {
//         start_date: i32,
//         end_date: Optional(i32) = .not_provided,
//         line: Optional(i32) = .not_provided,
//         shift: Optional(i32) = .not_provided,
//     },
//     all: struct {
//         line: Optional(i32) = .not_provided,
//         shift: Optional(i32) = .not_provided,
//     },
// }

describe(endpoint, () => {
  it("selects the `id` variant when only id is present", async () => {
    const res = await GET(endpoint, { queryParams: { id: 42 } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "id");
    assert.strictEqual(body.id, 42);
  });

  it("selects the `date_range` variant", async () => {
    const res = await GET(endpoint, { queryParams: { start_date: 123 } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "date_range");
    assert.strictEqual(body.start_date, 123);
  });

  it("selects `date_range` and parses its optional keys", async () => {
    const res = await GET(endpoint, {
      queryParams: { start_date: 123, end_date: 456, line: 1, shift: 2 },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "date_range");
    assert.strictEqual(body.start_date, 123);
    assert.strictEqual(body.end_date, 456);
    assert.strictEqual(body.line, 1);
    assert.strictEqual(body.shift, 2);
  });

  it("parses the optional end_date for the date_range variant", async () => {
    const res = await GET(endpoint, {
      queryParams: { start_date: 123, end_date: 456 },
    });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "date_range");
    assert.strictEqual(body.start_date, 123);
    assert.strictEqual(body.end_date, 456);
  });

  it("Selects the `all` variant", async () => {
    const res = await GET(endpoint, { queryParams: { line: 1, shift: 2 } });
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "all");
    assert.strictEqual(body.line, 1);
    assert.strictEqual(body.shift, 2);
  });

  it("resolves an empty query to the fallback variant", async () => {
    const res = await GET(endpoint);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.variant, "all");
    assert.strictEqual(body.line, null);
    assert.strictEqual(body.shift, null);
  });

  it("errors on an incorrect param value", async () => {
    const res = await GET(endpoint, "?start_date=invalid");
    assert.strictEqual(res.status, 400);
    assert.strictEqual(
      "Incorrect query parameter for: start_date",
      await res.text(),
    );
  });

  it("rejects an invalid end_date", async () => {
    const res = await GET(endpoint, "?start_date=123&end_date=invalid");
    assert.strictEqual(res.status, 400);
    assert.strictEqual(
      "Incorrect query parameter for: end_date",
      await res.text(),
    );
  });

  it("rejects query parameters from conflicting union variants", async () => {
    const res = await GET(endpoint, "?id=42&start_date=123");
    assert.strictEqual(res.status, 400);
    assert.strictEqual(
      "Unexpected query parameters were provided: start_date",
      await res.text(),
    );
  });
});
