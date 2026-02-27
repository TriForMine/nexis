# @triformine/nexis-sdk

TypeScript client SDK for Nexis multiplayer backend.

## Install

```bash
bun add @triformine/nexis-sdk
```

## Quick Start

```ts
import { connect } from "@triformine/nexis-sdk";

const client = await connect("ws://localhost:4000", {
  projectId: "<project-id>",
  token: "<client-token>",
});

const room = await client.joinOrCreate("counter_plugin_room", { roomId: "counter_plugin_room:default" });

room.onStateChange((state) => {
  console.log("state", state);
});

room.send("inc", { by: 1 });
```

## Common APIs

- `connect(url, options)`
- `client.joinOrCreate(roomType, options?)`
- `client.sendRPC(type, payload, room?)`
- `client.onEvent(type, callback)`
- `room.send(type, data)`
- `room.sendBytes(type, bytes)`
- `room.onMessage(type, callback)`
- `room.onStateChange(callback)`

## Docs

- Project docs: https://triformine.github.io/nexis/
- SDK docs: https://triformine.github.io/nexis/sdks/typescript/

## License

Apache-2.0
