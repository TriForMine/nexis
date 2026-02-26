import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { JsonCodec, MsgpackCodec } from "../src/codec";
import type { Envelope } from "../src/types";

const message = {
  v: 1,
  t: "state.patch",
  rid: "rid-1",
  room: "room-a",
  p: { counter: 2 },
};

function loadFixture(name: string): Envelope {
  const fixturePath = join(
    import.meta.dir,
    "../../../docs/fixtures/protocol",
    name,
  );
  return JSON.parse(readFileSync(fixturePath, "utf8")) as Envelope;
}

describe("codecs", () => {
  test("json encode/decode roundtrip", () => {
    const codec = new JsonCodec();
    const decoded = codec.decode(codec.encode(message));

    expect(decoded).toEqual(message);
  });

  test("msgpack encode/decode roundtrip", () => {
    const codec = new MsgpackCodec();
    const decoded = codec.decode(codec.encode(message));

    expect(decoded).toEqual(message);
  });

  test("fixture envelopes decode/encode in both codecs", () => {
    const fixtures = [
      "state_patch_v1_with_checksum.json",
      "state_patch_v1_without_checksum.json",
      "state_snapshot_v1.json",
    ].map(loadFixture);

    const jsonCodec = new JsonCodec();
    const msgpackCodec = new MsgpackCodec();

    for (const fixture of fixtures) {
      const fromJson = jsonCodec.decode(jsonCodec.encode(fixture));
      const fromMsgpack = msgpackCodec.decode(msgpackCodec.encode(fixture));

      expect(fromJson).toEqual(fixture);
      expect(fromMsgpack).toEqual(fixture);
    }
  });
});
