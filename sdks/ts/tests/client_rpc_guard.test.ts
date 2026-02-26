import { describe, expect, test } from "bun:test";
import { NexisClient } from "../src/client";
import { JsonCodec } from "../src/codec";
import { UnknownRidError } from "../src/rpc";

describe("NexisClient rpc.response guard", () => {
  test("does not throw when rpc.response has missing rid", async () => {
    const codec = new JsonCodec();
    const events: Array<{ t: string; p?: unknown }> = [];
    const fakeClient = {
      codec,
      rpc: {
        resolveResponse() {
          throw new UnknownRidError("missing");
        },
      },
      dispatchEvent(message: { t: string; p?: unknown }) {
        events.push(message);
      },
    } as unknown as NexisClient;

    const raw = codec.encode({
      v: 1,
      t: "rpc.response",
      room: "counter_plugin_room:default",
      p: { ok: true },
    });

    await expect(
      (NexisClient.prototype as unknown as { onRawMessage: (raw: unknown) => Promise<void> }).onRawMessage.call(
        fakeClient,
        raw,
      ),
    ).resolves.toBeUndefined();

    expect(events).toHaveLength(1);
    expect(events[0]?.t).toBe("error");
    expect(events[0]?.p).toEqual({ reason: "Unknown RPC rid: missing" });
  });
});
