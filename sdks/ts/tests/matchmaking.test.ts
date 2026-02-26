import { describe, expect, test } from "bun:test";
import { parseMatchFoundPayload } from "../src/client";

describe("matchmaking helpers", () => {
  test("parses valid match.found payload", () => {
    const parsed = parseMatchFoundPayload({
      room: "counter_room:match:abc123",
      room_type: "counter_room",
      size: 2,
      participants: ["s-1", "s-2"],
    });

    expect(parsed).toEqual({
      room: "counter_room:match:abc123",
      roomType: "counter_room",
      size: 2,
      participants: ["s-1", "s-2"],
    });
  });

  test("rejects malformed match.found payload", () => {
    expect(parseMatchFoundPayload({ room: 1 })).toBeNull();
  });
});
