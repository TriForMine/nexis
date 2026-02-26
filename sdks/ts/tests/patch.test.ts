import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  applyPatch,
  computeStateChecksum,
  parsePatchPayload,
  parseSnapshotPayload,
} from "../src/patch";
import type { PatchOp } from "../src/types";

function readProtocolFixture(name: string): unknown {
  const fixturePath = join(
    import.meta.dir,
    "../../../docs/fixtures/protocol",
    name,
  );
  return JSON.parse(readFileSync(fixturePath, "utf8")) as unknown;
}

describe("applyPatch", () => {
  test("applies set and del operations", () => {
    const state = { counter: 1, foo: true };
    const patch: PatchOp[] = [
      { op: "set", path: "/counter", value: 3 },
      { op: "set", path: "/bar", value: "x" },
      { op: "del", path: "/foo" },
    ];

    const result = applyPatch(state, patch);
    expect(result).toEqual({ counter: 3, bar: "x" });
  });

  test("parses legacy array patch payload", () => {
    const parsed = parsePatchPayload([
      { op: "set", path: "/counter", value: 2 },
    ]);
    expect(parsed).toEqual({
      seq: 0,
      checksum: undefined,
      ops: [{ op: "set", path: "/counter", value: 2 }],
    });
  });

  test("parses sequenced patch payload", () => {
    const parsed = parsePatchPayload({
      seq: 7,
      checksum: "abc",
      ops: [{ op: "set", path: "/counter", value: 8 }],
    });
    expect(parsed).toEqual({
      seq: 7,
      checksum: "abc",
      ops: [{ op: "set", path: "/counter", value: 8 }],
    });
  });

  test("parses sequenced patch payload without checksum", () => {
    const parsed = parsePatchPayload({
      seq: 8,
      ops: [{ op: "set", path: "/counter", value: 9 }],
    });
    expect(parsed).toEqual({
      seq: 8,
      checksum: undefined,
      ops: [{ op: "set", path: "/counter", value: 9 }],
    });
  });

  test("parses protocol fixtures for old/new patch payload variants", () => {
    const withChecksum = readProtocolFixture(
      "state_patch_v1_with_checksum.json",
    ) as { p?: unknown };
    const withoutChecksum = readProtocolFixture(
      "state_patch_v1_without_checksum.json",
    ) as { p?: unknown };

    const parsedWithChecksum = parsePatchPayload(withChecksum.p);
    const parsedWithoutChecksum = parsePatchPayload(withoutChecksum.p);

    expect(parsedWithChecksum?.seq).toBe(65);
    expect(typeof parsedWithChecksum?.checksum).toBe("string");
    expect(parsedWithoutChecksum?.seq).toBe(66);
    expect(parsedWithoutChecksum?.checksum).toBeUndefined();
  });

  test("parses snapshot payload", () => {
    const parsed = parseSnapshotPayload({
      seq: 10,
      checksum: "def",
      state: { counter: 10 },
    });
    expect(parsed).toEqual({
      seq: 10,
      checksum: "def",
      state: { counter: 10 },
    });
  });

  test("produces stable checksum for equivalent object ordering", async () => {
    const left = {
      b: 2,
      a: { z: true, k: [3, 2, 1] },
    };
    const right = {
      a: { k: [3, 2, 1], z: true },
      b: 2,
    };

    const leftChecksum = await computeStateChecksum(left);
    const rightChecksum = await computeStateChecksum(right);
    expect(leftChecksum).toBe(rightChecksum);
  });
});
