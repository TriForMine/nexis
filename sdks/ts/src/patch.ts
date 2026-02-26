import type { PatchOp, StatePatchPayload, StateSnapshotPayload } from "./types";

function keyFromPath(path: string): string {
  if (!path.startsWith("/")) {
    throw new Error(`Invalid patch path: ${path}`);
  }
  const key = path.slice(1);
  if (!key || key.includes("/")) {
    throw new Error(`Invalid patch path: ${path}`);
  }
  return key.replace(/~1/g, "/").replace(/~0/g, "~");
}

export function applyPatch<T extends Record<string, unknown>>(
  state: T,
  patch: PatchOp[],
): T {
  const next: Record<string, unknown> = { ...state };

  for (const op of patch) {
    const key = keyFromPath(op.path);
    if (op.op === "set") {
      next[key] = op.value;
      continue;
    }
    delete next[key];
  }

  return next as T;
}

function canonicalize(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => canonicalize(item));
  }

  if (value && typeof value === "object") {
    const sortedEntries = Object.entries(value as Record<string, unknown>).sort(
      ([left], [right]) => left.localeCompare(right),
    );
    const normalized: Record<string, unknown> = {};
    for (const [key, item] of sortedEntries) {
      normalized[key] = canonicalize(item);
    }
    return normalized;
  }

  return value;
}

export async function computeStateChecksum(state: unknown): Promise<string> {
  const cryptoApi = globalThis.crypto?.subtle;
  if (!cryptoApi) {
    throw new Error("crypto.subtle is unavailable");
  }

  const canonicalJson = JSON.stringify(canonicalize(state));
  const bytes = new TextEncoder().encode(canonicalJson);
  const digest = await cryptoApi.digest("SHA-256", bytes);
  const hashBytes = new Uint8Array(digest);

  return Array.from(hashBytes)
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
}

export function parsePatchPayload(payload: unknown): StatePatchPayload | null {
  if (Array.isArray(payload)) {
    return {
      seq: 0,
      checksum: undefined,
      ops: payload as PatchOp[],
    };
  }

  if (!payload || typeof payload !== "object") {
    return null;
  }

  const candidate = payload as {
    seq?: unknown;
    checksum?: unknown;
    ops?: unknown;
  };
  if (
    typeof candidate.seq !== "number" ||
    !Array.isArray(candidate.ops) ||
    (candidate.checksum !== undefined && typeof candidate.checksum !== "string")
  ) {
    return null;
  }

  return {
    seq: candidate.seq,
    checksum: candidate.checksum as string | undefined,
    ops: candidate.ops as PatchOp[],
  };
}

export function parseSnapshotPayload(
  payload: unknown,
): StateSnapshotPayload | null {
  if (!payload || typeof payload !== "object") {
    return null;
  }

  const candidate = payload as {
    seq?: unknown;
    checksum?: unknown;
    state?: unknown;
  };
  if (
    typeof candidate.seq !== "number" ||
    (candidate.checksum !== undefined &&
      typeof candidate.checksum !== "string") ||
    !candidate.state ||
    typeof candidate.state !== "object" ||
    Array.isArray(candidate.state)
  ) {
    return null;
  }

  return {
    seq: candidate.seq,
    checksum: candidate.checksum as string | undefined,
    state: candidate.state as Record<string, unknown>,
  };
}
