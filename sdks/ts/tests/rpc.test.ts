import { describe, expect, test } from "bun:test";
import { RpcClient, UnknownRidError } from "../src/rpc";

describe("RpcClient", () => {
  test("omits room field when request is global", () => {
    const rpc = new RpcClient();
    const { message } = rpc.createRequest("room.list", {
      roomType: "counter_plugin_room",
    });

    expect("room" in message).toBeFalse();
    expect(message).toMatchObject({
      v: 1,
      t: "room.list",
      rid: "rpc-1",
      p: { roomType: "counter_plugin_room" },
    });
  });

  test("resolves promise when rid matches", async () => {
    const rpc = new RpcClient();
    const { message, promise } = rpc.createRequest("room.join", { room: "r1" });

    rpc.resolveResponse({
      v: 1,
      t: "rpc.response",
      rid: message.rid,
      p: { ok: true },
    });

    await expect(promise).resolves.toEqual({ ok: true });
  });

  test("throws on unknown rid", () => {
    const rpc = new RpcClient();

    expect(() =>
      rpc.resolveResponse({ v: 1, t: "rpc.response", rid: "missing", p: null }),
    ).toThrowError(UnknownRidError);
  });

  test("throws on late duplicate response for already-resolved rid", async () => {
    const rpc = new RpcClient();
    const { message, promise } = rpc.createRequest("room.message", {
      type: "inc",
      data: { by: 1 },
    });

    rpc.resolveResponse({
      v: 1,
      t: "rpc.response",
      rid: message.rid,
      p: { ok: true },
    });
    await expect(promise).resolves.toEqual({ ok: true });

    expect(() =>
      rpc.resolveResponse({
        v: 1,
        t: "rpc.response",
        rid: message.rid,
        p: { ok: true },
      }),
    ).toThrowError(UnknownRidError);
  });
});
