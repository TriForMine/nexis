import type { Envelope } from "./types";

export class UnknownRidError extends Error {
  constructor(rid: string) {
    super(`Unknown RPC rid: ${rid}`);
  }
}

export class RpcClient {
  private nextId = 1;
  private pending = new Map<
    string,
    { resolve: (payload: unknown) => void; reject: (error: Error) => void }
  >();

  createRequest(
    type: string,
    payload: unknown,
    room?: string,
  ): { message: Envelope; promise: Promise<unknown> } {
    const rid = `rpc-${this.nextId++}`;
    const message: Envelope = { v: 1, t: type, rid, p: payload };
    if (room !== undefined) {
      message.room = room;
    }
    const promise = new Promise<unknown>((resolve, reject) => {
      this.pending.set(rid, { resolve, reject });
    });

    return { message, promise };
  }

  resolveResponse(response: Envelope): void {
    const rid = response.rid;
    if (!rid) {
      throw new UnknownRidError("missing");
    }

    const pending = this.pending.get(rid);
    if (!pending) {
      throw new UnknownRidError(rid);
    }

    this.pending.delete(rid);
    pending.resolve(response.p);
  }

  rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }
}
