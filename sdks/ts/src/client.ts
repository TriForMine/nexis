import {
  applyPatch,
  computeStateChecksum,
  parsePatchPayload,
  parseSnapshotPayload,
} from "./patch";
import { JsonCodec, MsgpackCodec, codecFor, type Codec } from "./codec";
import { RpcClient, UnknownRidError } from "./rpc";
import type {
  ConnectOptions,
  Envelope,
  HandshakeRequest,
  MatchFound,
  MatchmakingDequeueResponse,
  MatchmakingQueueResponse,
  RoomListResponse,
  RoomMessagePayload,
  RoomMessageType,
} from "./types";

const DEFAULT_VERSION = 1;
const DEFAULT_RECONNECT_INITIAL_DELAY_MS = 250;
const DEFAULT_RECONNECT_MAX_DELAY_MS = 3_000;
const DEFAULT_RECONNECT_MAX_ATTEMPTS = 20;

type EventHandler = (message: Envelope) => void;
type StateHandler = (state: Record<string, unknown>) => void;
type MatchFoundHandler = (match: MatchFound, message: Envelope) => void;
type RoomMessageHandler = (data: unknown, envelope: Envelope) => void;
type SelectorHandler = (value: unknown, state: Record<string, unknown>) => void;
type SupportedCodec = "json" | "msgpack";

type SelectorRegistration = {
  path: string;
  callback: SelectorHandler;
  lastValue: unknown;
};

type ResolvedConnectOptions = ConnectOptions & {
  codecs: Array<"msgpack" | "json">;
  autoJoinMatchedRoom: boolean;
  autoReconnect: boolean;
  reconnectInitialDelayMs: number;
  reconnectMaxDelayMs: number;
  reconnectMaxAttempts: number;
};

export type RoomStateChangeSubscription = {
  (callback: StateHandler): () => void;
  once(callback: StateHandler): () => void;
  select(path: string, callback: SelectorHandler): () => void;
};

function normalizeConnectOptions(
  options: ConnectOptions,
): ResolvedConnectOptions {
  const reconnectInitialDelayMs = Math.max(
    50,
    options.reconnectInitialDelayMs ?? DEFAULT_RECONNECT_INITIAL_DELAY_MS,
  );
  const reconnectMaxDelayMs = Math.max(
    reconnectInitialDelayMs,
    options.reconnectMaxDelayMs ?? DEFAULT_RECONNECT_MAX_DELAY_MS,
  );
  const reconnectMaxAttempts = Math.max(
    1,
    options.reconnectMaxAttempts ?? DEFAULT_RECONNECT_MAX_ATTEMPTS,
  );
  return {
    ...options,
    codecs: options.codecs ?? ["msgpack", "json"],
    autoJoinMatchedRoom: options.autoJoinMatchedRoom ?? false,
    autoReconnect: options.autoReconnect ?? true,
    reconnectInitialDelayMs,
    reconnectMaxDelayMs,
    reconnectMaxAttempts,
  };
}

function readCodecName(message: Envelope): SupportedCodec {
  const payload = message.p;
  if (
    payload &&
    typeof payload === "object" &&
    "codec" in payload &&
    (payload as { codec?: unknown }).codec === "msgpack"
  ) {
    return "msgpack";
  }
  return "json";
}

function readSessionId(message: Envelope): string | undefined {
  const payload = message.p;
  if (
    payload &&
    typeof payload === "object" &&
    "session_id" in payload &&
    typeof (payload as { session_id?: unknown }).session_id === "string"
  ) {
    return (payload as { session_id: string }).session_id;
  }
  return undefined;
}

function readErrorReason(message: Envelope): string {
  const payload = message.p;
  if (
    payload &&
    typeof payload === "object" &&
    "reason" in payload &&
    typeof (payload as { reason?: unknown }).reason === "string"
  ) {
    return (payload as { reason: string }).reason;
  }
  return "server returned error";
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function deepEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function toJsonValue(bytes: Uint8Array): string {
  let binary = "";
  for (const value of bytes) {
    binary += String.fromCharCode(value);
  }
  return btoa(binary);
}

function fromJsonValue(value: unknown): Uint8Array | null {
  if (typeof value !== "string") {
    return null;
  }

  try {
    const decoded = atob(value);
    const bytes = new Uint8Array(decoded.length);
    for (let i = 0; i < decoded.length; i += 1) {
      bytes[i] = decoded.charCodeAt(i);
    }
    return bytes;
  } catch {
    return null;
  }
}

function readRoomMessagePayload(payload: unknown): RoomMessagePayload | null {
  if (!payload || typeof payload !== "object") {
    return null;
  }
  const type = (payload as { type?: unknown }).type;
  if (typeof type !== "string" && typeof type !== "number") {
    return null;
  }
  const data = (payload as { data?: unknown }).data;
  return { type, data };
}

export function parseMatchFoundPayload(payload: unknown): MatchFound | null {
  if (!payload || typeof payload !== "object") {
    return null;
  }

  const room = (payload as { room?: unknown }).room;
  const roomType = (payload as { room_type?: unknown }).room_type;
  const size = (payload as { size?: unknown }).size;
  const participants = (payload as { participants?: unknown }).participants;
  if (
    typeof room !== "string" ||
    typeof roomType !== "string" ||
    typeof size !== "number" ||
    !Number.isFinite(size) ||
    !isStringArray(participants)
  ) {
    return null;
  }

  return {
    room,
    roomType,
    size,
    participants,
  };
}

export class NexisRoom {
  readonly id: string;
  readonly onStateChange: RoomStateChangeSubscription;

  constructor(
    private readonly client: NexisClient,
    roomId: string,
  ) {
    this.id = roomId;
    this.onStateChange = this.buildStateChangeSubscription();
  }

  get state(): Record<string, unknown> {
    return this.client.getRoomState(this.id);
  }

  send(type: RoomMessageType, message: unknown): void {
    this.client.sendRoomMessage(this.id, type, message);
  }

  sendBytes(type: RoomMessageType, bytes: Uint8Array | number[]): void {
    const normalized =
      bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
    this.client.sendRoomMessageBytes(this.id, type, normalized);
  }

  onMessage(type: RoomMessageType, callback: RoomMessageHandler): () => void {
    return this.client.onRoomMessage(this.id, type, callback);
  }

  private buildStateChangeSubscription(): RoomStateChangeSubscription {
    const subscribe = (callback: StateHandler): (() => void) =>
      this.client.onRoomState(this.id, callback);

    subscribe.once = (callback: StateHandler): (() => void) =>
      this.client.onRoomStateOnce(this.id, callback);

    subscribe.select = (
      path: string,
      callback: SelectorHandler,
    ): (() => void) => this.client.onRoomStateSelect(this.id, path, callback);

    return subscribe;
  }
}

export class NexisClient {
  private socket: WebSocket;
  private readonly url: string;
  private readonly connectOptions: ResolvedConnectOptions;
  private readonly rpc = new RpcClient();
  private codec: Codec;
  private readonly eventHandlers = new Map<string, Set<EventHandler>>();
  private readonly stateHandlers = new Set<StateHandler>();
  private readonly roomStateHandlers = new Map<string, Set<StateHandler>>();
  private readonly roomStateSelectors = new Map<
    string,
    Set<SelectorRegistration>
  >();
  private readonly roomMessageHandlers = new Map<
    string,
    Map<string, Set<RoomMessageHandler>>
  >();
  private readonly roomStates = new Map<string, Record<string, unknown>>();
  private readonly roomSeq = new Map<string, number>();
  private readonly roomChecksum = new Map<string, string>();
  private sessionId: string | undefined;
  private readonly autoJoinMatchedRoom: boolean;
  private reconnecting = false;
  private disposed = false;

  private constructor(
    url: string,
    socket: WebSocket,
    codec: Codec,
    sessionId: string | undefined,
    options: ResolvedConnectOptions,
  ) {
    this.url = url;
    this.socket = socket;
    this.codec = codec;
    this.sessionId = sessionId;
    this.connectOptions = options;
    this.autoJoinMatchedRoom = options.autoJoinMatchedRoom;
    this.attachSocket(socket);
  }

  static connect(url: string, options: ConnectOptions): Promise<NexisClient> {
    const resolved = normalizeConnectOptions(options);
    return NexisClient.openSocketAndHandshake(
      url,
      resolved,
      resolved.sessionId,
    ).then(
      ({ socket, codec, sessionId }) =>
        new NexisClient(url, socket, codec, sessionId, resolved),
    );
  }

  close(): void {
    this.disposed = true;
    this.socket.close();
  }

  private attachSocket(socket: WebSocket): void {
    this.socket = socket;
    this.socket.addEventListener("message", (event) => {
      void this.onRawMessage(event.data);
    });
    this.socket.addEventListener("close", () => {
      this.rpc.rejectAll(new Error("socket closed"));
      if (!this.disposed && this.connectOptions.autoReconnect) {
        void this.tryReconnect();
      }
    });
  }

  private async tryReconnect(): Promise<void> {
    if (this.reconnecting || this.disposed) {
      return;
    }
    this.reconnecting = true;
    this.dispatchEvent({
      v: DEFAULT_VERSION,
      t: "reconnect.start",
      p: { session_id: this.sessionId },
    });

    let attempt = 0;
    let delayMs = this.connectOptions.reconnectInitialDelayMs;
    while (
      !this.disposed &&
      attempt < this.connectOptions.reconnectMaxAttempts
    ) {
      attempt += 1;
      await NexisClient.wait(delayMs);
      try {
        const reconnect = await NexisClient.openSocketAndHandshake(
          this.url,
          this.connectOptions,
          this.sessionId,
        );
        this.codec = reconnect.codec;
        this.sessionId = reconnect.sessionId ?? this.sessionId;
        this.attachSocket(reconnect.socket);
        this.dispatchEvent({
          v: DEFAULT_VERSION,
          t: "reconnect.ok",
          p: { attempt, session_id: this.sessionId },
        });
        this.reconnecting = false;
        return;
      } catch {
        this.dispatchEvent({
          v: DEFAULT_VERSION,
          t: "reconnect.retry",
          p: { attempt },
        });
      }
      delayMs = Math.min(delayMs * 2, this.connectOptions.reconnectMaxDelayMs);
    }

    this.reconnecting = false;
    this.dispatchEvent({
      v: DEFAULT_VERSION,
      t: "reconnect.failed",
      p: { session_id: this.sessionId },
    });
  }

  getSessionId(): string | undefined {
    return this.sessionId;
  }

  room(roomId: string): NexisRoom {
    return new NexisRoom(this, roomId);
  }

  async joinOrCreate(
    roomType: string,
    options?: { roomId?: string } & Record<string, unknown>,
  ): Promise<NexisRoom> {
    const roomId =
      typeof options?.roomId === "string" ? options.roomId : undefined;
    const response = await this.sendRPC(
      "room.join_or_create",
      {
        roomType,
        roomId,
        options,
      },
      roomId,
    );
    if (
      response &&
      typeof response === "object" &&
      typeof (response as { room?: unknown }).room === "string"
    ) {
      return this.room((response as { room: string }).room);
    }
    if (roomId) {
      return this.room(roomId);
    }
    return this.room(`${roomType}:default`);
  }

  listRooms(roomType?: string): Promise<RoomListResponse> {
    return this.sendRPC(
      "room.list",
      roomType ? { roomType } : {},
    ) as Promise<RoomListResponse>;
  }

  enqueueMatchmaking(
    roomType: string,
    size = 2,
  ): Promise<MatchmakingQueueResponse> {
    return this.sendRPC("matchmaking.enqueue", {
      roomType,
      size,
    }) as Promise<MatchmakingQueueResponse>;
  }

  dequeueMatchmaking(): Promise<MatchmakingDequeueResponse> {
    return this.sendRPC(
      "matchmaking.dequeue",
      {},
    ) as Promise<MatchmakingDequeueResponse>;
  }

  onStateChange(callback: StateHandler): () => void {
    this.stateHandlers.add(callback);
    return () => this.stateHandlers.delete(callback);
  }

  onEvent(type: string, callback: EventHandler): () => void {
    const handlers = this.eventHandlers.get(type) ?? new Set<EventHandler>();
    handlers.add(callback);
    this.eventHandlers.set(type, handlers);
    return () => {
      const current = this.eventHandlers.get(type);
      if (!current) {
        return;
      }
      current.delete(callback);
      if (current.size === 0) {
        this.eventHandlers.delete(type);
      }
    };
  }

  onMatchFound(callback: MatchFoundHandler): () => void {
    return this.onEvent("match.found", (message) => {
      const parsed = parseMatchFoundPayload(message.p);
      if (!parsed) {
        return;
      }
      callback(parsed, message);
    });
  }

  sendRPC(type: string, payload: unknown, room?: string): Promise<unknown> {
    const { message, promise } = this.rpc.createRequest(type, payload, room);
    this.sendEnvelope(message);
    return promise;
  }

  getRoomState(roomId: string): Record<string, unknown> {
    return this.roomStates.get(roomId) ?? {};
  }

  sendRoomMessage(roomId: string, type: RoomMessageType, data: unknown): void {
    this.sendEnvelope({
      v: DEFAULT_VERSION,
      t: "room.message",
      room: roomId,
      p: { type: String(type), data },
    });
  }

  sendRoomMessageBytes(
    roomId: string,
    type: RoomMessageType,
    data: Uint8Array,
  ): void {
    this.sendEnvelope({
      v: DEFAULT_VERSION,
      t: "room.message.bytes",
      room: roomId,
      p: { type: String(type), data_b64: toJsonValue(data) },
    });
  }

  onRoomMessage(
    roomId: string,
    type: RoomMessageType,
    callback: RoomMessageHandler,
  ): () => void {
    const key = String(type);
    const byType = this.roomMessageHandlers.get(roomId) ?? new Map();
    const handlers = byType.get(key) ?? new Set<RoomMessageHandler>();
    handlers.add(callback);
    byType.set(key, handlers);
    this.roomMessageHandlers.set(roomId, byType);

    return () => {
      const roomHandlers = this.roomMessageHandlers.get(roomId);
      if (!roomHandlers) {
        return;
      }
      const typeHandlers = roomHandlers.get(key);
      if (!typeHandlers) {
        return;
      }
      typeHandlers.delete(callback);
      if (typeHandlers.size === 0) {
        roomHandlers.delete(key);
      }
      if (roomHandlers.size === 0) {
        this.roomMessageHandlers.delete(roomId);
      }
    };
  }

  onRoomState(roomId: string, callback: StateHandler): () => void {
    const handlers =
      this.roomStateHandlers.get(roomId) ?? new Set<StateHandler>();
    handlers.add(callback);
    this.roomStateHandlers.set(roomId, handlers);
    if (this.roomStates.has(roomId)) {
      callback(this.roomStates.get(roomId) ?? {});
    }
    return () => {
      const current = this.roomStateHandlers.get(roomId);
      if (!current) {
        return;
      }
      current.delete(callback);
      if (current.size === 0) {
        this.roomStateHandlers.delete(roomId);
      }
    };
  }

  onRoomStateOnce(roomId: string, callback: StateHandler): () => void {
    let disposed = false;
    const off = this.onRoomState(roomId, (state) => {
      if (disposed) {
        return;
      }
      disposed = true;
      off();
      callback(state);
    });
    return () => {
      disposed = true;
      off();
    };
  }

  onRoomStateSelect(
    roomId: string,
    path: string,
    callback: SelectorHandler,
  ): () => void {
    const normalizedPath = path.startsWith("/") ? path.slice(1) : path;
    const currentState = this.getRoomState(roomId);
    const registration: SelectorRegistration = {
      path: normalizedPath,
      callback,
      lastValue: currentState[normalizedPath],
    };
    const selectors = this.roomStateSelectors.get(roomId) ?? new Set();
    selectors.add(registration);
    this.roomStateSelectors.set(roomId, selectors);

    return () => {
      const current = this.roomStateSelectors.get(roomId);
      if (!current) {
        return;
      }
      current.delete(registration);
      if (current.size === 0) {
        this.roomStateSelectors.delete(roomId);
      }
    };
  }

  private sendEnvelope(message: Envelope): void {
    const bytes = this.codec.encode(message);
    this.socket.send(bytes);
  }

  private dispatchEvent(message: Envelope): void {
    const handlers = this.eventHandlers.get(message.t);
    if (!handlers) {
      return;
    }

    for (const handler of handlers) {
      handler(message);
    }
  }

  private dispatchState(
    roomId: string,
    nextState: Record<string, unknown>,
    prevState: Record<string, unknown>,
  ): void {
    for (const handler of this.stateHandlers) {
      handler(nextState);
    }

    const roomHandlers = this.roomStateHandlers.get(roomId);
    if (roomHandlers) {
      for (const handler of roomHandlers) {
        handler(nextState);
      }
    }

    const selectors = this.roomStateSelectors.get(roomId);
    if (selectors) {
      for (const registration of selectors) {
        const nextValue = nextState[registration.path];
        const prevValue = prevState[registration.path];
        if (!deepEqual(nextValue, prevValue)) {
          registration.lastValue = nextValue;
          registration.callback(nextValue, nextState);
        }
      }
    }
  }

  private dispatchRoomMessage(message: Envelope): void {
    if (!message.room) {
      return;
    }

    const payload = readRoomMessagePayload(message.p);
    if (!payload) {
      return;
    }

    const byType = this.roomMessageHandlers.get(message.room);
    if (!byType) {
      return;
    }

    const handlers = byType.get(String(payload.type));
    if (!handlers) {
      return;
    }
    for (const handler of handlers) {
      handler(payload.data, message);
    }
  }

  private async onRawMessage(raw: unknown): Promise<void> {
    const bytes = await NexisClient.toBytes(raw);
    if (!bytes) {
      return;
    }

    let message: Envelope;
    try {
      message = this.codec.decode(bytes);
    } catch {
      return;
    }

    if (message.t === "rpc.response") {
      try {
        this.rpc.resolveResponse(message);
      } catch (error) {
        if (error instanceof UnknownRidError) {
          this.dispatchEvent({
            v: DEFAULT_VERSION,
            t: "error",
            p: { reason: error.message },
          });
          return;
        }
        throw error;
      }
      return;
    }

    if (message.t === "state.snapshot") {
      if (!message.room) {
        return;
      }
      const snapshot = parseSnapshotPayload(message.p);
      if (!snapshot) {
        return;
      }
      const computedChecksum = await computeStateChecksum(snapshot.state);
      if (snapshot.checksum && snapshot.checksum !== computedChecksum) {
        this.sendEnvelope({
          v: DEFAULT_VERSION,
          t: "state.resync",
          room: message.room,
          p: { since: this.roomSeq.get(message.room) ?? 0 },
        });
        return;
      }
      const checksum = snapshot.checksum ?? computedChecksum;

      const prevState = this.roomStates.get(message.room) ?? {};
      this.roomStates.set(message.room, snapshot.state);
      this.roomSeq.set(message.room, snapshot.seq);
      this.roomChecksum.set(message.room, checksum);
      this.dispatchState(message.room, snapshot.state, prevState);
      this.sendEnvelope({
        v: DEFAULT_VERSION,
        t: "state.ack",
        room: message.room,
        p: { seq: snapshot.seq, checksum },
      });
      return;
    }

    if (message.t === "state.patch") {
      if (!message.room) {
        return;
      }
      const parsedPatch = parsePatchPayload(message.p);
      if (!parsedPatch) {
        return;
      }

      const currentSeq = this.roomSeq.get(message.room) ?? 0;
      const patchSeq = parsedPatch.seq > 0 ? parsedPatch.seq : currentSeq + 1;

      if (patchSeq <= currentSeq) {
        return;
      }

      if (patchSeq > currentSeq + 1) {
        this.sendEnvelope({
          v: DEFAULT_VERSION,
          t: "state.resync",
          room: message.room,
          p: { since: currentSeq },
        });
        return;
      }

      const currentState = this.roomStates.get(message.room) ?? {};
      const nextState = applyPatch(currentState, parsedPatch.ops);
      let localChecksum: string | undefined;
      if (parsedPatch.checksum) {
        localChecksum = await computeStateChecksum(nextState);
        if (parsedPatch.checksum !== localChecksum) {
          this.sendEnvelope({
            v: DEFAULT_VERSION,
            t: "state.resync",
            room: message.room,
            p: {
              since: currentSeq,
              checksum: this.roomChecksum.get(message.room),
            },
          });
          return;
        }
      }

      this.roomStates.set(message.room, nextState);
      this.roomSeq.set(message.room, patchSeq);
      if (parsedPatch.checksum) {
        this.roomChecksum.set(message.room, parsedPatch.checksum);
      } else if (localChecksum) {
        this.roomChecksum.set(message.room, localChecksum);
      }
      this.dispatchState(message.room, nextState, currentState);
      this.sendEnvelope({
        v: DEFAULT_VERSION,
        t: "state.ack",
        room: message.room,
        p: parsedPatch.checksum
          ? { seq: patchSeq, checksum: this.roomChecksum.get(message.room) }
          : { seq: patchSeq },
      });
      return;
    }

    if (this.autoJoinMatchedRoom && message.t === "match.found") {
      const parsed = parseMatchFoundPayload(message.p);
      if (parsed) {
        void this.joinOrCreate(parsed.roomType, { roomId: parsed.room }).catch(
          () => undefined,
        );
      }
    }

    if (message.t === "room.message" && message.room) {
      this.dispatchRoomMessage(message);
    }

    this.dispatchEvent(message);
  }

  private static openSocketAndHandshake(
    url: string,
    options: ResolvedConnectOptions,
    sessionIdOverride?: string,
  ): Promise<{
    socket: WebSocket;
    codec: Codec;
    sessionId: string | undefined;
  }> {
    const socket = new WebSocket(url);
    const jsonCodec = new JsonCodec();
    const msgpackCodec = new MsgpackCodec();

    return new Promise((resolve, reject) => {
      let settled = false;
      const handshakeSessionId = sessionIdOverride ?? options.sessionId;

      const onOpen = () => {
        const handshake: HandshakeRequest = {
          v: DEFAULT_VERSION,
          codecs: options.codecs,
          project_id: options.projectId?.trim() || "anonymous",
          token: options.token?.trim() || "",
          session_id: handshakeSessionId,
        };
        socket.send(JSON.stringify(handshake));
      };

      const onError = () => {
        finishReject(new Error("socket connection failed"));
      };

      const onClose = () => {
        finishReject(new Error("socket closed before handshake"));
      };

      const onMessage = async (event: MessageEvent) => {
        try {
          const message = await NexisClient.decodeHandshakeMessage(
            event.data,
            jsonCodec,
            msgpackCodec,
          );
          if (!message) {
            return;
          }

          if (message.t === "error") {
            finishReject(new Error(readErrorReason(message)));
            return;
          }

          if (message.t !== "handshake.ok") {
            return;
          }

          const negotiatedCodec = readCodecName(message);
          const sessionId = readSessionId(message) ?? handshakeSessionId;
          finishResolve({
            socket,
            codec: codecFor(negotiatedCodec),
            sessionId,
          });
        } catch (error) {
          finishReject(new Error(`handshake decode failed: ${String(error)}`));
        }
      };

      const cleanup = () => {
        socket.removeEventListener("open", onOpen);
        socket.removeEventListener("error", onError);
        socket.removeEventListener("close", onClose);
        socket.removeEventListener("message", onMessage);
      };

      const finishResolve = (result: {
        socket: WebSocket;
        codec: Codec;
        sessionId: string | undefined;
      }) => {
        if (settled) {
          return;
        }
        settled = true;
        cleanup();
        resolve(result);
      };

      const finishReject = (error: Error) => {
        if (settled) {
          return;
        }
        settled = true;
        cleanup();
        reject(error);
      };

      socket.addEventListener("open", onOpen);
      socket.addEventListener("error", onError);
      socket.addEventListener("close", onClose);
      socket.addEventListener("message", onMessage);
    });
  }

  private static wait(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  private static async decodeHandshakeMessage(
    raw: unknown,
    jsonCodec: JsonCodec,
    msgpackCodec: MsgpackCodec,
  ): Promise<Envelope | null> {
    if (typeof raw === "string") {
      return JSON.parse(raw) as Envelope;
    }

    const bytes = await NexisClient.toBytes(raw);
    if (!bytes) {
      return null;
    }

    try {
      return msgpackCodec.decode(bytes);
    } catch {
      return jsonCodec.decode(bytes);
    }
  }

  private static async toBytes(raw: unknown): Promise<Uint8Array | null> {
    if (raw instanceof Uint8Array) {
      return raw;
    }
    if (raw instanceof ArrayBuffer) {
      return new Uint8Array(raw);
    }
    if (raw instanceof Blob) {
      return new Uint8Array(await raw.arrayBuffer());
    }
    if (typeof raw === "string") {
      return new TextEncoder().encode(raw);
    }
    return null;
  }
}

export async function connect(
  url: string,
  options: ConnectOptions,
): Promise<NexisClient> {
  return NexisClient.connect(url, options);
}

export function decodeRoomBytes(payload: unknown): Uint8Array | null {
  return fromJsonValue(payload);
}
