import { describe, expect, test } from "bun:test";
import { NexisClient } from "../src/client";

describe("NexisClient room state subscriptions", () => {
  test("onRoomState immediately emits cached state when available", () => {
    const callbacks: Array<Record<string, unknown>> = [];
    const fakeClient = {
      roomStateHandlers: new Map<string, Set<(state: Record<string, unknown>) => void>>(),
      roomStates: new Map<string, Record<string, unknown>>([
        ["counter_plugin_room:default", { counter: 7 }],
      ]),
    } as unknown as NexisClient;

    (
      NexisClient.prototype as unknown as {
        onRoomState: (
          roomId: string,
          callback: (state: Record<string, unknown>) => void,
        ) => () => void;
      }
    ).onRoomState.call(fakeClient, "counter_plugin_room:default", (state) => {
      callbacks.push(state);
    });

    expect(callbacks).toEqual([{ counter: 7 }]);
  });
});
