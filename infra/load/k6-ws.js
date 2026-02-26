import { check } from "k6";
import ws from "k6/ws";
import { Counter, Trend } from "k6/metrics";

const wsErrors = new Counter("nexis_ws_errors");
const handshakeLatency = new Trend("nexis_handshake_latency_ms");
const joinLatency = new Trend("nexis_join_latency_ms");
const roomMessageRtt = new Trend("nexis_room_message_rtt_ms");
const statePatchCount = new Counter("nexis_state_patch_count");

export const options = {
  vus: Number(__ENV.VUS ?? 20),
  duration: __ENV.DURATION ?? "30s",
  thresholds: {
    nexis_ws_errors: ["count==0"],
    nexis_handshake_latency_ms: ["p(95)<500", "p(99)<1000"],
    nexis_join_latency_ms: ["p(95)<700", "p(99)<1500"],
    nexis_room_message_rtt_ms: ["p(95)<700", "p(99)<1500"],
  },
};

const WS_URL = __ENV.NEXIS_WS_URL ?? "ws://localhost:4000";
const PROJECT_ID = __ENV.NEXIS_PROJECT_ID ?? "demo-project";
const TOKEN = __ENV.NEXIS_TOKEN ?? "";
const ROOM_ID_BASE = __ENV.NEXIS_ROOM_ID ?? "counter_plugin_room:load";
const ROOM_SHARDS = Math.max(1, Number(__ENV.ROOM_SHARDS ?? 1));
const INC_INTERVAL_MS = Number(__ENV.INC_INTERVAL_MS ?? 250);

export default function () {
  const roomId =
    ROOM_SHARDS > 1
      ? `${ROOM_ID_BASE}:${(__VU - 1) % ROOM_SHARDS}`
      : ROOM_ID_BASE;
  const startedAt = Date.now();
  let handshakeAt = 0;
  let joinedAt = 0;
  const pendingInc = new Map();

  const response = ws.connect(WS_URL, {}, (socket) => {
    socket.on("open", () => {
      socket.send(
        JSON.stringify({
          v: 1,
          codecs: ["json"],
          project_id: PROJECT_ID,
          token: TOKEN,
        }),
      );
    });

    socket.on("message", (raw) => {
      let message;
      try {
        message = JSON.parse(raw);
      } catch (_) {
        wsErrors.add(1);
        return;
      }

      if (message.t === "error") {
        wsErrors.add(1);
        socket.close();
        return;
      }

      if (message.t === "handshake.ok") {
        handshakeAt = Date.now();
        handshakeLatency.add(handshakeAt - startedAt);

        socket.send(
          JSON.stringify({
            v: 1,
            t: "room.join_or_create",
            rid: "join-1",
            room: roomId,
            p: { roomType: "counter_plugin_room", roomId: roomId },
          }),
        );
        return;
      }

      if (message.t === "rpc.response" && message.rid === "join-1") {
        joinedAt = Date.now();
        joinLatency.add(joinedAt - handshakeAt);

        socket.setInterval(() => {
          const rid = `inc-${Date.now()}-${Math.random()}`;
          pendingInc.set(rid, Date.now());
          socket.send(
            JSON.stringify({
              v: 1,
              t: "room.message",
              rid,
              room: roomId,
              p: { type: "inc", data: { by: 1 } },
            }),
          );
        }, INC_INTERVAL_MS);
        return;
      }

      if (message.t === "rpc.response" && typeof message.rid === "string") {
        if (message.rid.startsWith("inc-")) {
          const sentAt = pendingInc.get(message.rid);
          if (sentAt) {
            roomMessageRtt.add(Date.now() - sentAt);
            pendingInc.delete(message.rid);
          }
        }
        return;
      }

      if (message.t === "state.patch") {
        statePatchCount.add(1);
      }
    });

    socket.on("error", () => {
      wsErrors.add(1);
    });

    socket.setTimeout(() => {
      socket.close();
    }, 25_000);
  });

  check(response, {
    "ws handshake is HTTP 101": (res) => res && res.status === 101,
  });
}
