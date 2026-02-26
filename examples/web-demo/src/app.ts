import { connect, type NexisClient, type NexisRoom } from "@nexis/sdk-ts";

type TokenClaims = {
  project_id?: string;
};

const wsUrlInput = document.getElementById("wsUrl") as HTMLInputElement;
const projectIdInput = document.getElementById("projectId") as HTMLInputElement;
const tokenInput = document.getElementById("token") as HTMLInputElement;
const connectButton = document.getElementById("connect") as HTMLButtonElement;
const incButton = document.getElementById("inc") as HTMLButtonElement;
const listRoomsButton = document.getElementById(
  "listRooms",
) as HTMLButtonElement;
const enqueueMatchButton = document.getElementById(
  "enqueueMatch",
) as HTMLButtonElement;
const dequeueMatchButton = document.getElementById(
  "dequeueMatch",
) as HTMLButtonElement;
const counterEl = document.getElementById("counter") as HTMLDivElement;
const logEl = document.getElementById("log") as HTMLPreElement;
const SESSION_STORAGE_KEY = "nexis.demo.session_id";

let client: NexisClient | null = null;
let room: NexisRoom | null = null;

function log(value: unknown): void {
  const line =
    typeof value === "string" ? value : JSON.stringify(value, null, 2);
  logEl.textContent = `${new Date().toISOString()} ${line}\n\n${logEl.textContent}`;
}

function decodeTokenClaims(token: string): TokenClaims | null {
  const parts = token.split(".");
  if (parts.length !== 2) {
    return null;
  }

  try {
    const base64 = parts[0].replace(/-/g, "+").replace(/_/g, "/");
    const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
    const json = atob(padded);
    return JSON.parse(json) as TokenClaims;
  } catch {
    return null;
  }
}

function syncProjectFromToken(): void {
  const claims = decodeTokenClaims(tokenInput.value.trim());
  if (!claims?.project_id) {
    return;
  }

  if (projectIdInput.value.trim() !== claims.project_id) {
    projectIdInput.value = claims.project_id;
    log(`project_id updated from token claims: ${claims.project_id}`);
  }
}

tokenInput.addEventListener("input", syncProjectFromToken);

connectButton.addEventListener("click", async () => {
  if (client) {
    log("already connected");
    return;
  }

  const wsUrl = wsUrlInput.value.trim();
  const token = tokenInput.value.trim();
  syncProjectFromToken();
  const projectId = projectIdInput.value.trim();

  if (!wsUrl) {
    log("ws url is required");
    return;
  }

  connectButton.disabled = true;
  incButton.disabled = true;
  listRoomsButton.disabled = true;
  enqueueMatchButton.disabled = true;
  dequeueMatchButton.disabled = true;

  try {
    const resumeSessionId =
      localStorage.getItem(SESSION_STORAGE_KEY) ?? undefined;
    client = await connect(wsUrl, {
      projectId: projectId || undefined,
      token: token || undefined,
      sessionId: resumeSessionId,
      autoJoinMatchedRoom: true,
    });

    log("connected and handshake completed");
    const sessionId = client.getSessionId();
    if (sessionId) {
      localStorage.setItem(SESSION_STORAGE_KEY, sessionId);
      log({ t: "session", session_id: sessionId });
    }

    client.onEvent("error", (message) => {
      log(message);
    });

    client.onEvent("room.members", (message) => {
      log(message);
    });
    client.onMatchFound((match, message) => {
      log({ t: "match.found", match, raw: message });
    });

    client.onEvent("room.message", (message) => {
      log({ t: "room.message", p: message.p });
    });

    const joined = await client.joinOrCreate("counter_plugin_room", {
      roomId: "counter_plugin_room:default",
    });
    room = joined;

    room.onStateChange((nextState) => {
      const counter =
        typeof nextState.counter === "number" ? nextState.counter : 0;
      counterEl.textContent = String(counter);
      log({ t: "state", counter });
    });
    room.onMessage("counter.updated", (messagePayload) => {
      log({ t: "counter.updated", p: messagePayload });
    });

    log({ t: "join.ok", room: room.id });

    incButton.disabled = false;
    listRoomsButton.disabled = false;
    enqueueMatchButton.disabled = false;
    dequeueMatchButton.disabled = false;
  } catch (error) {
    client = null;
    log({ t: "connect.error", reason: String(error) });
  } finally {
    connectButton.disabled = false;
  }
});

incButton.addEventListener("click", async () => {
  if (!client) {
    log("not connected");
    return;
  }
  if (!room) {
    log("not in room");
    return;
  }

  try {
    room.send("inc", { by: 1 });
    log({ t: "room.send", p: { type: "inc", by: 1 } });
  } catch (error) {
    log({ t: "room.send.error", reason: String(error) });
    incButton.disabled = true;
    listRoomsButton.disabled = true;
    enqueueMatchButton.disabled = true;
    dequeueMatchButton.disabled = true;
    client = null;
    room = null;
  }
});

listRoomsButton.addEventListener("click", async () => {
  if (!client) {
    log("not connected");
    return;
  }

  try {
    const response = await client.listRooms("counter_plugin_room");
    log({ t: "room.list.ok", p: response });
  } catch (error) {
    log({ t: "room.list.error", reason: String(error) });
  }
});

enqueueMatchButton.addEventListener("click", async () => {
  if (!client) {
    log("not connected");
    return;
  }

  try {
    const response = await client.enqueueMatchmaking("counter_plugin_room", 2);
    log({ t: "matchmaking.enqueue.ok", p: response });
  } catch (error) {
    log({ t: "matchmaking.enqueue.error", reason: String(error) });
  }
});

dequeueMatchButton.addEventListener("click", async () => {
  if (!client) {
    log("not connected");
    return;
  }

  try {
    const response = await client.dequeueMatchmaking();
    log({ t: "matchmaking.dequeue.ok", p: response });
  } catch (error) {
    log({ t: "matchmaking.dequeue.error", reason: String(error) });
  }
});
