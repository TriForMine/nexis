type Envelope = {
  v: number;
  t: string;
  rid?: string;
  room?: string;
  p?: unknown;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  const bodyText = await response.text();
  let payload: unknown = null;
  if (bodyText.length > 0) {
    try {
      payload = JSON.parse(bodyText) as unknown;
    } catch {
      throw new Error(
        `HTTP ${response.status} for ${url}: expected JSON body, got ${bodyText}`,
      );
    }
  }
  if (!response.ok) {
    throw new Error(
      `HTTP ${response.status} for ${url}: ${JSON.stringify(payload)}`,
    );
  }
  return payload as T;
}

class JsonSocketSession {
  private readonly socket: WebSocket;
  private readonly queue: Envelope[] = [];
  private readonly waiters: Array<{
    predicate: (message: Envelope) => boolean;
    resolve: (message: Envelope) => void;
    reject: (error: Error) => void;
    timer: ReturnType<typeof setTimeout>;
  }> = [];

  constructor(socket: WebSocket) {
    this.socket = socket;
    this.socket.addEventListener("message", (event) => {
      if (typeof event.data !== "string") {
        return;
      }
      let message: Envelope;
      try {
        message = JSON.parse(event.data) as Envelope;
      } catch {
        return;
      }
      const waiterIndex = this.waiters.findIndex((waiter) =>
        waiter.predicate(message),
      );
      if (waiterIndex >= 0) {
        const [waiter] = this.waiters.splice(waiterIndex, 1);
        clearTimeout(waiter.timer);
        waiter.resolve(message);
        return;
      }
      this.queue.push(message);
    });

    this.socket.addEventListener("close", () => {
      const pending = this.waiters.splice(0, this.waiters.length);
      for (const waiter of pending) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error("websocket closed"));
      }
    });
  }

  send(message: Envelope): void {
    this.socket.send(JSON.stringify(message));
  }

  waitFor(
    predicate: (message: Envelope) => boolean,
    timeoutMs: number,
  ): Promise<Envelope> {
    const queuedIndex = this.queue.findIndex(predicate);
    if (queuedIndex >= 0) {
      const [message] = this.queue.splice(queuedIndex, 1);
      return Promise.resolve(message);
    }

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const index = this.waiters.findIndex(
          (item) => item.resolve === resolve,
        );
        if (index >= 0) {
          this.waiters.splice(index, 1);
        }
        reject(new Error("timed out waiting for websocket message"));
      }, timeoutMs);

      this.waiters.push({ predicate, resolve, reject, timer });
    });
  }
}

async function openWebSocket(
  url: string,
  timeoutMs: number,
): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const timer = setTimeout(() => {
      reject(new Error(`timed out connecting to ${url}`));
    }, timeoutMs);

    socket.addEventListener("open", () => {
      clearTimeout(timer);
      resolve(socket);
    });
    socket.addEventListener("error", () => {
      clearTimeout(timer);
      reject(new Error(`websocket error connecting to ${url}`));
    });
  });
}

async function assertHandshakeDenied(
  url: string,
  handshake: {
    v: number;
    codecs: string[];
    project_id: string;
    token: string;
  },
  timeoutMs: number,
): Promise<void> {
  const socket = await openWebSocket(url, timeoutMs);
  const outcome = await new Promise<"denied" | "accepted" | "timeout">(
    (resolve) => {
      const timer = setTimeout(() => resolve("timeout"), timeoutMs);
      socket.addEventListener("message", (event) => {
        if (typeof event.data !== "string") {
          return;
        }
        try {
          const envelope = JSON.parse(event.data) as Envelope;
          if (envelope.t === "handshake.ok") {
            clearTimeout(timer);
            resolve("accepted");
          }
        } catch {
          // ignore malformed payloads
        }
      });
      socket.addEventListener("close", () => {
        clearTimeout(timer);
        resolve("denied");
      });
      socket.addEventListener("error", () => {
        clearTimeout(timer);
        resolve("denied");
      });
      socket.send(JSON.stringify(handshake));
    },
  );

  if (socket.readyState === WebSocket.OPEN) {
    socket.close();
  }

  assert(outcome === "denied", "revoked token handshake should be denied");
}

const controlApiUrl =
  process.env.NEXIS_CONTROL_API_URL ?? "http://127.0.0.1:3000";
const wsUrl = process.env.NEXIS_WS_URL ?? "ws://127.0.0.1:4000";
const timeoutMs = Number(process.env.NEXIS_SMOKE_TIMEOUT_MS ?? 10_000);
const sessionTtlSeconds = Number(process.env.NEXIS_SESSION_TTL_SECONDS ?? 30);
const resumeExpiryWaitMs = Math.max(1, sessionTtlSeconds + 1) * 1_000;

const projectName = `smoke-${Date.now()}`;

console.log(`[smoke] creating project "${projectName}"`);
const project = await requestJson<{
  id: string;
  name: string;
}>(`${controlApiUrl}/projects`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ name: projectName }),
});

console.log(`[smoke] creating key for project ${project.id}`);
const key = await requestJson<{
  id: string;
}>(`${controlApiUrl}/projects/${project.id}/keys`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ name: "smoke" }),
});

console.log(`[smoke] minting token`);
const minted = await requestJson<{
  token: string;
  claims: {
    project_id: string;
    issued_at: string;
    expires_at: string;
  };
}>(`${controlApiUrl}/tokens`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    project_id: project.id,
    key_id: key.id,
    ttl_seconds: 300,
  }),
});

assert(
  minted.claims.project_id === project.id,
  "minted claims project_id mismatch",
);

console.log("[smoke] rotating key and checking hardening");
const rotatedKey = await requestJson<{ id: string }>(
  `${controlApiUrl}/projects/${project.id}/keys/${key.id}/rotate`,
  {
    method: "POST",
  },
);

const oldMintAttempt = await fetch(`${controlApiUrl}/tokens`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    project_id: project.id,
    key_id: key.id,
    ttl_seconds: 300,
  }),
});
assert(
  oldMintAttempt.status === 403,
  "old key should be revoked after rotation",
);

const rotatedMinted = await requestJson<{
  token: string;
  claims: {
    project_id: string;
    issued_at: string;
    expires_at: string;
    key_id?: string;
  };
}>(`${controlApiUrl}/tokens`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    project_id: project.id,
    key_id: rotatedKey.id,
    ttl_seconds: 300,
    audience: wsUrl,
  }),
});
assert(
  rotatedMinted.claims.key_id === rotatedKey.id,
  "rotated key token should contain key_id claim",
);

console.log("[smoke] ensuring rotated-away token is denied by data plane");
await assertHandshakeDenied(
  wsUrl,
  {
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: minted.token,
  },
  Math.min(timeoutMs, 4_000),
);

console.log(`[smoke] opening websocket ${wsUrl}`);
const socket = await openWebSocket(wsUrl, timeoutMs);
const session = new JsonSocketSession(socket);

socket.send(
  JSON.stringify({
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: rotatedMinted.token,
  }),
);

const handshake = await session.waitFor(
  (message) => message.t === "handshake.ok",
  timeoutMs,
);
assert(handshake.v === 1, "handshake version mismatch");
assert(
  typeof handshake.p === "object" &&
    handshake.p !== null &&
    (handshake.p as { codec?: unknown }).codec === "json",
  "expected json codec negotiation",
);
assert(
  typeof (handshake.p as { session_id?: unknown }).session_id === "string",
  "expected handshake session_id",
);
const sessionId = (handshake.p as { session_id: string }).session_id;

const room = "counter_plugin_room:smoke";
session.send({
  v: 1,
  t: "room.join_or_create",
  rid: "join-1",
  room,
  p: { roomType: "counter_plugin_room", roomId: room },
});

const joinResponse = await session.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "join-1",
  timeoutMs,
);
assert(
  typeof joinResponse.p === "object" &&
    joinResponse.p !== null &&
    (joinResponse.p as { ok?: unknown }).ok === true,
  "join_or_create did not succeed",
);

const initialSnapshot = await session.waitFor(
  (message) => message.t === "state.snapshot" && message.room === room,
  timeoutMs,
);
assert(
  typeof initialSnapshot.p === "object" &&
    initialSnapshot.p !== null &&
    typeof (initialSnapshot.p as { seq?: unknown }).seq === "number" &&
    typeof (initialSnapshot.p as { checksum?: unknown }).checksum ===
      "string" &&
    typeof (initialSnapshot.p as { state?: unknown }).state === "object",
  "state.snapshot payload must include seq, checksum, and state",
);

session.send({
  v: 1,
  t: "room.message",
  rid: "inc-1",
  room,
  p: { type: "inc", data: { by: 1 } },
});

const incResponse = await session.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "inc-1",
  timeoutMs,
);
assert(
  typeof incResponse.p === "object" &&
    incResponse.p !== null &&
    (incResponse.p as { ok?: unknown }).ok === true,
  "room.message inc response was not ok",
);

const patch = await session.waitFor(
  (message) => message.t === "state.patch" && message.room === room,
  timeoutMs,
);
assert(
  typeof patch.p === "object" &&
    patch.p !== null &&
    typeof (patch.p as { seq?: unknown }).seq === "number" &&
    ((patch.p as { checksum?: unknown }).checksum === undefined ||
      typeof (patch.p as { checksum?: unknown }).checksum === "string") &&
    Array.isArray((patch.p as { ops?: unknown }).ops),
  "state.patch payload must include seq, ops array, and optional checksum",
);

const patchPayload = patch.p as {
  seq: number;
  checksum?: string;
};

console.log("[smoke] testing checksum mismatch recovery");
session.send({
  v: 1,
  t: "state.ack",
  room,
  p: { seq: patchPayload.seq, checksum: "0000000000000000" },
});
const mismatchSnapshot = await session.waitFor(
  (message) => message.t === "state.snapshot" && message.room === room,
  timeoutMs,
);
assert(
  typeof mismatchSnapshot.p === "object" &&
    mismatchSnapshot.p !== null &&
    typeof (mismatchSnapshot.p as { checksum?: unknown }).checksum === "string",
  "checksum mismatch should trigger state.snapshot recovery",
);

console.log("[smoke] testing explicit resync loop stability");
for (let i = 0; i < 3; i += 1) {
  session.send({
    v: 1,
    t: "state.resync",
    room,
    p: { since: patchPayload.seq },
  });
  const resyncSnapshot = await session.waitFor(
    (message) => message.t === "state.snapshot" && message.room === room,
    timeoutMs,
  );
  assert(
    typeof resyncSnapshot.p === "object" &&
      resyncSnapshot.p !== null &&
      typeof (resyncSnapshot.p as { seq?: unknown }).seq === "number" &&
      typeof (resyncSnapshot.p as { checksum?: unknown }).checksum === "string",
    "state.resync should return stable state.snapshot",
  );
}

session.send({
  v: 1,
  t: "room.list",
  rid: "list-1",
  p: { roomType: "counter_plugin_room" },
});

const roomList = await session.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "list-1",
  timeoutMs,
);
assert(
  typeof roomList.p === "object" &&
    roomList.p !== null &&
    (roomList.p as { ok?: unknown }).ok === true,
  "room.list should succeed",
);
const listedRooms = (roomList.p as { rooms?: unknown }).rooms;
assert(Array.isArray(listedRooms), "room.list should return rooms array");
assert(
  listedRooms.some((entry) => {
    if (!entry || typeof entry !== "object") {
      return false;
    }
    const roomId = (entry as { id?: unknown }).id;
    return roomId === room;
  }),
  "room.list should include joined counter plugin room",
);

socket.close();
await Bun.sleep(150);

console.log("[smoke] reconnecting with session resume");
const resumedSocket = await openWebSocket(wsUrl, timeoutMs);
const resumedSession = new JsonSocketSession(resumedSocket);
resumedSocket.send(
  JSON.stringify({
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: rotatedMinted.token,
    session_id: sessionId,
  }),
);

const resumedHandshake = await resumedSession.waitFor(
  (message) => message.t === "handshake.ok",
  timeoutMs,
);
assert(
  typeof resumedHandshake.p === "object" &&
    resumedHandshake.p !== null &&
    (resumedHandshake.p as { resumed?: unknown }).resumed === true,
  "expected resumed handshake",
);
const resumedSessionId = (resumedHandshake.p as { session_id?: unknown })
  .session_id as string | undefined;
assert(typeof resumedSessionId === "string", "resumed session_id missing");

await resumedSession.waitFor(
  (message) => message.t === "state.snapshot" && message.room === room,
  timeoutMs,
);

resumedSession.send({
  v: 1,
  t: "room.message",
  rid: "inc-2",
  room,
  p: { type: "inc", data: { by: 1 } },
});

const resumedInc = await resumedSession.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "inc-2",
  timeoutMs,
);
assert(
  typeof resumedInc.p === "object" &&
    resumedInc.p !== null &&
    (resumedInc.p as { ok?: unknown }).ok === true,
  "room.message inc after resume was not ok",
);

console.log("[smoke] testing matchmaking");
const peerSocket = await openWebSocket(wsUrl, timeoutMs);
const peerSession = new JsonSocketSession(peerSocket);
peerSocket.send(
  JSON.stringify({
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: rotatedMinted.token,
  }),
);
const peerHandshake = await peerSession.waitFor(
  (message) => message.t === "handshake.ok",
  timeoutMs,
);
const peerSessionId = (peerHandshake.p as { session_id?: unknown })
  .session_id as string | undefined;
assert(typeof peerSessionId === "string", "peer session_id missing");

resumedSession.send({
  v: 1,
  t: "matchmaking.enqueue",
  rid: "mm-1",
  p: { roomType: "counter_plugin_room", size: 2 },
});
await resumedSession.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "mm-1",
  timeoutMs,
);

peerSession.send({
  v: 1,
  t: "matchmaking.enqueue",
  rid: "mm-2",
  p: { roomType: "counter_plugin_room", size: 2 },
});
await peerSession.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "mm-2",
  timeoutMs,
);

const matchFromResumed = await resumedSession.waitFor(
  (message) => message.t === "match.found",
  timeoutMs,
);
const matchFromPeer = await peerSession.waitFor(
  (message) => message.t === "match.found",
  timeoutMs,
);
assert(
  matchFromResumed.room === matchFromPeer.room,
  "matched room should match",
);

const participants = (matchFromResumed.p as { participants?: unknown })
  .participants;
assert(Array.isArray(participants), "match.found should include participants");
assert(
  participants.includes(resumedSessionId) &&
    participants.includes(peerSessionId),
  "match participants should include both session ids",
);

const metrics = await requestJson<{
  counters: Record<string, number>;
}>(`${controlApiUrl}/metrics`);
assert(
  metrics.counters.keys_rotated >= 1,
  "metrics should count key rotations",
);
assert(metrics.counters.tokens_minted >= 2, "metrics should count token mints");
assert(metrics.counters.requests_total >= 1, "metrics should count requests");

resumedSocket.close();
peerSocket.close();

console.log("[smoke] testing resume-after-expiry");
const expiringSocket = await openWebSocket(wsUrl, timeoutMs);
const expiringSession = new JsonSocketSession(expiringSocket);
expiringSocket.send(
  JSON.stringify({
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: rotatedMinted.token,
  }),
);
const expiringHandshake = await expiringSession.waitFor(
  (message) => message.t === "handshake.ok",
  timeoutMs,
);
const expiringSessionId = (expiringHandshake.p as { session_id?: unknown })
  .session_id as string | undefined;
assert(
  typeof expiringSessionId === "string",
  "expiring session handshake should include session_id",
);
expiringSession.send({
  v: 1,
  t: "room.join_or_create",
  rid: "join-expiry",
  room: "echo_room:expiry",
  p: { roomType: "echo_room", roomId: "echo_room:expiry" },
});
await expiringSession.waitFor(
  (message) => message.t === "rpc.response" && message.rid === "join-expiry",
  timeoutMs,
);
expiringSocket.close();

await Bun.sleep(resumeExpiryWaitMs);

const expiredResumeSocket = await openWebSocket(wsUrl, timeoutMs);
const expiredResumeSession = new JsonSocketSession(expiredResumeSocket);
expiredResumeSocket.send(
  JSON.stringify({
    v: 1,
    codecs: ["json"],
    project_id: project.id,
    token: rotatedMinted.token,
    session_id: expiringSessionId,
  }),
);
const expiredResumeHandshake = await expiredResumeSession.waitFor(
  (message) => message.t === "handshake.ok",
  timeoutMs,
);
assert(
  typeof expiredResumeHandshake.p === "object" &&
    expiredResumeHandshake.p !== null &&
    (expiredResumeHandshake.p as { resumed?: unknown }).resumed === false,
  "expired session resume should not be accepted",
);
const replacementSessionId = (
  expiredResumeHandshake.p as { session_id?: unknown }
).session_id as string | undefined;
assert(
  typeof replacementSessionId === "string" &&
    replacementSessionId !== expiringSessionId,
  "expired resume should issue a fresh session_id",
);
expiredResumeSocket.close();

console.log("[smoke] success");
