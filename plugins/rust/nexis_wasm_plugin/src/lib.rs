use serde_json::Value;
use std::slice;

pub type JsonValue = Value;

#[derive(Debug, Clone)]
pub struct PluginOutput {
    pub state: JsonValue,
    pub event: Option<JsonValue>,
}

impl PluginOutput {
    pub fn state(state: JsonValue) -> Self {
        Self { state, event: None }
    }

    pub fn state_with_event(state: JsonValue, event: JsonValue) -> Self {
        Self {
            state,
            event: Some(event),
        }
    }
}

pub trait RoomPlugin {
    fn initial_state(&self) -> JsonValue;

    fn on_message(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }

    fn on_create(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }

    fn on_join(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }

    fn on_leave(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }

    fn on_tick(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }

    fn on_dispose(&self, state: &JsonValue, _input: &JsonValue) -> PluginOutput {
        PluginOutput::state(state.clone())
    }
}

#[doc(hidden)]
pub fn __pack_ptr_len(ptr: *const u8, len: usize) -> i64 {
    ((ptr as i64) << 32) | ((len as u32) as i64)
}

#[doc(hidden)]
pub fn __encode_json(value: JsonValue) -> i64 {
    let mut bytes = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let ptr = bytes.as_mut_ptr();
    let len = bytes.len();
    std::mem::forget(bytes);
    __pack_ptr_len(ptr.cast_const(), len)
}

#[doc(hidden)]
pub fn __decode_json(ptr: i32, len: i32) -> JsonValue {
    if ptr < 0 || len <= 0 {
        return JsonValue::Null;
    }
    // WASM ABI is host-managed; host owns source buffer lifetime.
    let bytes = unsafe { slice::from_raw_parts(ptr as *const u8, len as usize) };
    serde_json::from_slice::<JsonValue>(bytes).unwrap_or(JsonValue::Null)
}

#[doc(hidden)]
pub fn __alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }
    let mut bytes = vec![0_u8; len as usize];
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    ptr as i32
}

#[macro_export]
macro_rules! export_plugin {
    ($plugin_ty:ty) => {
        fn __nexis_plugin() -> &'static $plugin_ty {
            static INSTANCE: std::sync::OnceLock<$plugin_ty> = std::sync::OnceLock::new();
            INSTANCE.get_or_init(|| <$plugin_ty as Default>::default())
        }

        fn __nexis_invoke<F>(state_ptr: i32, state_len: i32, input_ptr: i32, input_len: i32, hook: F) -> i64
        where
            F: FnOnce(&$plugin_ty, &$crate::JsonValue, &$crate::JsonValue) -> $crate::PluginOutput,
        {
            let plugin = __nexis_plugin();
            let state = $crate::__decode_json(state_ptr, state_len);
            let input = $crate::__decode_json(input_ptr, input_len);
            let output = hook(plugin, &state, &input);
            $crate::__encode_json(serde_json::json!({
                "state": output.state,
                "event": output.event
            }))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn alloc(len: i32) -> i32 {
            $crate::__alloc(len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_initial_state() -> i64 {
            let plugin = __nexis_plugin();
            $crate::__encode_json(<$plugin_ty as $crate::RoomPlugin>::initial_state(plugin))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_message(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_message(plugin, state, input)
            })
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_create(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_create(plugin, state, input)
            })
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_join(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_join(plugin, state, input)
            })
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_leave(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_leave(plugin, state, input)
            })
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_tick(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_tick(plugin, state, input)
            })
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn nexis_on_dispose(
            state_ptr: i32,
            state_len: i32,
            input_ptr: i32,
            input_len: i32,
        ) -> i64 {
            __nexis_invoke(state_ptr, state_len, input_ptr, input_len, |plugin, state, input| {
                <$plugin_ty as $crate::RoomPlugin>::on_dispose(plugin, state, input)
            })
        }
    };
}
