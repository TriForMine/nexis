# WASM Plugin Examples

## Rust Counter Plugin

Path: `examples/wasm-plugins/counter_rust_plugin`

This example uses Nexis' Rust helper crate:

`plugins/rust/nexis_wasm_plugin`

so plugin authors write normal Rust handlers instead of raw ABI exports.

Build:

```bash
cd examples/wasm-plugins/counter_rust_plugin
cargo build --target wasm32-unknown-unknown --release
```

Output:

`target/wasm32-unknown-unknown/release/nexis_counter_plugin.wasm`

Use with Nexis:

```bash
NEXIS_WASM_ROOM_PLUGINS='{"counter_plugin_room":"/app/plugins/nexis_counter_plugin.wasm"}'
```

Then invoke room logic with message type `room.message` and payload:

```json
{
  "type": "inc",
  "data": { "by": 1 }
}
```

Minimal plugin authoring shape:

```rust
use nexis_wasm_plugin::{export_plugin, PluginOutput, RoomPlugin};
use serde_json::{json, Value};

#[derive(Default)]
struct MyPlugin;

impl RoomPlugin for MyPlugin {
    fn initial_state(&self) -> Value {
        json!({ "counter": 0 })
    }

    fn on_message(&self, state: &Value, input: &Value) -> PluginOutput {
        PluginOutput::state(state.clone())
    }
}

export_plugin!(MyPlugin);
```
