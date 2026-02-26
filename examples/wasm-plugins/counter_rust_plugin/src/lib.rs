use nexis_wasm_plugin::{export_plugin, PluginOutput, RoomPlugin};
use serde_json::{json, Value};

#[derive(Default)]
struct CounterPlugin;

impl RoomPlugin for CounterPlugin {
    fn initial_state(&self) -> Value {
        json!({ "counter": 0 })
    }

    fn on_message(&self, state: &Value, input: &Value) -> PluginOutput {
        let current = state.get("counter").and_then(Value::as_i64).unwrap_or(0);
        let by = if input.get("type").and_then(Value::as_str) == Some("inc") {
            input
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("by"))
                .and_then(Value::as_i64)
                .unwrap_or(1)
        } else {
            0
        };
        let next = current.saturating_add(by);

        PluginOutput::state_with_event(
            json!({ "counter": next }),
            json!({ "type": "counter.updated", "data": { "by": by, "counter": next } }),
        )
    }
}

export_plugin!(CounterPlugin);
