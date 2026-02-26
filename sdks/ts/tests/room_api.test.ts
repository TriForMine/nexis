import { describe, expect, test } from "bun:test";
import { decodeRoomBytes, NexisRoom, type NexisClient } from "../src/client";

describe("NexisRoom", () => {
  test("send delegates room-scoped message dispatch", () => {
    const calls: Array<{ roomId: string; type: string | number; data: unknown }> = [];
    const client = {
      sendRoomMessage(roomId: string, type: string | number, data: unknown) {
        calls.push({ roomId, type, data });
      },
      sendRoomMessageBytes() {
        throw new Error("not used");
      },
      onRoomMessage() {
        return () => undefined;
      },
      onRoomState() {
        return () => undefined;
      },
      onRoomStateOnce() {
        return () => undefined;
      },
      onRoomStateSelect() {
        return () => undefined;
      },
      getRoomState() {
        return {};
      },
    } as unknown as NexisClient;

    const room = new NexisRoom(client, "room-a");
    room.send("move", { x: 1, y: 2 });

    expect(calls).toEqual([
      {
        roomId: "room-a",
        type: "move",
        data: { x: 1, y: 2 },
      },
    ]);
  });

  test("sendBytes encodes as base64 payload", () => {
    const calls: Array<{ roomId: string; type: string | number; data: Uint8Array }> = [];
    const client = {
      sendRoomMessage() {
        throw new Error("not used");
      },
      sendRoomMessageBytes(roomId: string, type: string | number, data: Uint8Array) {
        calls.push({ roomId, type, data });
      },
      onRoomMessage() {
        return () => undefined;
      },
      onRoomState() {
        return () => undefined;
      },
      onRoomStateOnce() {
        return () => undefined;
      },
      onRoomStateSelect() {
        return () => undefined;
      },
      getRoomState() {
        return {};
      },
    } as unknown as NexisClient;

    const room = new NexisRoom(client, "room-a");
    room.sendBytes("bytes", [1, 2, 3, 255]);

    expect(calls).toHaveLength(1);
    expect(Array.from(calls[0]!.data)).toEqual([1, 2, 3, 255]);
  });

  test("decodeRoomBytes decodes base64 payload", () => {
    const bytes = decodeRoomBytes("AQID/w==");
    expect(bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(bytes ?? new Uint8Array())).toEqual([1, 2, 3, 255]);
  });
});
