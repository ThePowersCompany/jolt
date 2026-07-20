import assert from "node:assert";
import { describe, it } from "node:test";
import { GET } from "../utils/client";

const endpoint = "/param-parse";

// Testing:
// const Timestamp = struct {
//     millis: i64,
//
//     pub fn paramParse(_: Allocator, param: []const u8) !Timestamp {
//         return .{ .millis = try std.fmt.parseInt(i64, param, 10) };
//     }
// };
//
// const GetContext = struct {
//     query_params: struct {
//         when: Optional(Timestamp) = .not_provided,
//     },
// };
//

describe(endpoint, () => {
  it("runs paramParse and echoes the parsed value", async () => {
    const millis = 1700000000000;
    const res = await GET(`${endpoint}?when=${millis}`, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.when_present, true);
    assert.strictEqual(body.millis, millis);
  });

  it("treats the scalar as absent when omitted", async () => {
    const res = await GET(endpoint, null);
    if (res.status != 200) {
      assert.fail(await res.text());
    }
    const body = await res.json();
    assert.strictEqual(body.when_present, false);
    assert.strictEqual(body.millis, null);
  });

  it("rejects a value that paramParse cannot parse", async () => {
    const res = await GET(`${endpoint}?when=abc`, null);
    assert.strictEqual(res.status, 400);
  });
});
