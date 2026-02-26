import { Packr, Unpackr } from "msgpackr";
import type { Envelope } from "./types";

export interface Codec {
  readonly name: "json" | "msgpack";
  encode(message: Envelope): Uint8Array;
  decode(bytes: Uint8Array): Envelope;
}

export class JsonCodec implements Codec {
  readonly name = "json" as const;

  encode(message: Envelope): Uint8Array {
    return new TextEncoder().encode(JSON.stringify(message));
  }

  decode(bytes: Uint8Array): Envelope {
    const text = new TextDecoder().decode(bytes);
    const parsed = JSON.parse(text);
    return parsed as Envelope;
  }
}

export class MsgpackCodec implements Codec {
  readonly name = "msgpack" as const;
  private readonly packr = new Packr({
    useRecords: false,
    structuredClone: false,
    bundleStrings: false,
    maxSharedStructures: 0,
  });
  private readonly unpackr = new Unpackr({
    useRecords: false,
  });

  encode(message: Envelope): Uint8Array {
    return this.packr.pack(message);
  }

  decode(bytes: Uint8Array): Envelope {
    const decoded = this.unpackr.unpack(bytes);
    return decoded as Envelope;
  }
}

export function codecFor(name: "json" | "msgpack"): Codec {
  return name === "msgpack" ? new MsgpackCodec() : new JsonCodec();
}
