# WASM Room Plugins (Beta)

Nexis can load room logic from WASM binaries at startup without rebuilding the Rust server.

## Configure

Set `NEXIS_WASM_ROOM_PLUGINS` to a JSON map:

```bash
NEXIS_WASM_ROOM_PLUGINS='{"duel_room":"/app/plugins/duel_room.wasm"}'
```

In Docker Compose, `../plugins` is mounted to `/app/plugins` in the data-plane container.

Reference plugin example:

- `examples/wasm-plugins/counter_rust_plugin`

## Runtime behavior

- Room type is auto-registered when plugin loads.
- Initial room state comes from plugin export `nexis_initial_state`.
- Clients invoke plugin logic with message type `room.message`.
- Server applies plugin-returned state, emits `state.patch`/`state.snapshot` as needed, and may emit room-scoped `room.message`.

## Plugin ABI

The module must export:

- `memory` (linear memory)
- `alloc(len: i32) -> i32`
- `nexis_initial_state() -> i64`
- optional lifecycle hooks using the same signature:
  - `nexis_on_create(state_ptr, state_len, input_ptr, input_len) -> i64`
  - `nexis_on_join(state_ptr, state_len, input_ptr, input_len) -> i64`
  - `nexis_on_message(state_ptr, state_len, input_ptr, input_len) -> i64`
  - `nexis_on_tick(state_ptr, state_len, input_ptr, input_len) -> i64`
  - `nexis_on_leave(state_ptr, state_len, input_ptr, input_len) -> i64`
  - `nexis_on_dispose(state_ptr, state_len, input_ptr, input_len) -> i64`

`i64` return format is packed pointer/length:

- high 32 bits: pointer
- low 32 bits: length

Payload contracts:

- `nexis_initial_state` returns UTF-8 JSON of room state (`serde_json::Value`)
- hook returns UTF-8 JSON:

```json
{
  "state": {},
  "event": {}
}
```

`state` and `event` are optional. Missing hook exports are treated as no-op.

## Call example

```json
{
  "v": 1,
  "t": "room.message",
  "rid": "plugin-1",
  "room": "duel_room:default",
  "p": { "type": "attack", "data": { "target": "p2", "damage": 8 } }
}
```

## Example plugin test

Run the real example plugin compile + runtime test:

```bash
cd server
NEXIS_RUN_EXAMPLE_WASM_TEST=1 cargo test -p runtime tests::rust_example_wasm_plugin_compiles_and_runs_when_enabled -- --exact
```
