export type Envelope = {
  v: number;
  t: string;
  rid?: string;
  room?: string;
  p?: unknown;
};

export type HandshakeRequest = {
  v: number;
  codecs: string[];
  project_id: string;
  token: string;
  session_id?: string;
};

export type ConnectOptions = {
  projectId?: string;
  token?: string;
  codecs?: Array<"msgpack" | "json">;
  sessionId?: string;
  autoJoinMatchedRoom?: boolean;
  autoReconnect?: boolean;
  reconnectInitialDelayMs?: number;
  reconnectMaxDelayMs?: number;
  reconnectMaxAttempts?: number;
};

export type RoomSummary = {
  id: string;
  room_type: string;
  members: number;
};

export type RoomListResponse = {
  ok: boolean;
  rooms: RoomSummary[];
};

export type MatchFound = {
  room: string;
  roomType: string;
  size: number;
  participants: string[];
};

export type MatchmakingQueueResponse = {
  ok: boolean;
  queued?: boolean;
  matched?: boolean;
  room_type?: string;
  size?: number;
  position?: number;
};

export type MatchmakingDequeueResponse = {
  ok: boolean;
  removed: boolean;
};

export type PatchOp =
  | { op: "set"; path: string; value: unknown }
  | { op: "del"; path: string };

export type StatePatchPayload = {
  seq: number;
  checksum?: string;
  ops: PatchOp[];
};

export type StateSnapshotPayload = {
  seq: number;
  checksum?: string;
  state: Record<string, unknown>;
};

export type RoomMessageType = string | number;

export type RoomMessagePayload = {
  type: RoomMessageType;
  data: unknown;
};
