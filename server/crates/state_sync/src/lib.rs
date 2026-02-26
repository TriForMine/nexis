use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op")]
pub enum PatchOp {
    #[serde(rename = "set")]
    Set { path: String, value: Value },
    #[serde(rename = "del")]
    Del { path: String },
}

#[derive(Debug, Error, PartialEq)]
pub enum PatchError {
    #[error("invalid path")]
    InvalidPath,
}

fn pointer_for_key(key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("/{escaped}")
}

fn key_from_path(path: &str) -> Result<Option<String>, PatchError> {
    if path.is_empty() {
        return Ok(None);
    }

    if !path.starts_with('/') {
        return Err(PatchError::InvalidPath);
    }

    let raw = &path[1..];
    if raw.is_empty() || raw.contains('/') {
        return Err(PatchError::InvalidPath);
    }

    Ok(Some(raw.replace("~1", "/").replace("~0", "~")))
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();

            let mut normalized = serde_json::Map::new();
            for key in keys {
                if let Some(item) = map.get(&key) {
                    normalized.insert(key, canonicalize(item));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

pub fn state_checksum(state: &Value) -> String {
    let canonical = canonicalize(state);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest = Sha256::digest(bytes);

    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

pub fn diff(old: &Value, new: &Value) -> Vec<PatchOp> {
    match (old.as_object(), new.as_object()) {
        (Some(old_obj), Some(new_obj)) => {
            let keys: BTreeSet<&str> = old_obj
                .keys()
                .map(String::as_str)
                .chain(new_obj.keys().map(String::as_str))
                .collect();

            let mut patches = Vec::new();
            for key in keys {
                match (old_obj.get(key), new_obj.get(key)) {
                    (None, Some(new_val)) => patches.push(PatchOp::Set {
                        path: pointer_for_key(key),
                        value: new_val.clone(),
                    }),
                    (Some(_), None) => patches.push(PatchOp::Del {
                        path: pointer_for_key(key),
                    }),
                    (Some(old_val), Some(new_val)) if old_val != new_val => {
                        patches.push(PatchOp::Set {
                            path: pointer_for_key(key),
                            value: new_val.clone(),
                        });
                    }
                    _ => {}
                }
            }
            patches
        }
        _ if old != new => vec![PatchOp::Set {
            path: "".to_owned(),
            value: new.clone(),
        }],
        _ => Vec::new(),
    }
}

pub fn apply_patch(state: &mut Value, patches: &[PatchOp]) -> Result<(), PatchError> {
    for patch in patches {
        match patch {
            PatchOp::Set { path, value } => match key_from_path(path)? {
                None => *state = value.clone(),
                Some(key) => {
                    let obj = state.as_object_mut().ok_or(PatchError::InvalidPath)?;
                    obj.insert(key, value.clone());
                }
            },
            PatchOp::Del { path } => match key_from_path(path)? {
                None => *state = Value::Null,
                Some(key) => {
                    let obj = state.as_object_mut().ok_or(PatchError::InvalidPath)?;
                    obj.remove(&key);
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec::Codec;
    use codec_json::JsonCodec;
    use codec_msgpack::MsgpackCodec;
    use protocol::{Message, PROTOCOL_VERSION};
    use serde_json::json;

    #[test]
    fn state_transition_generates_deterministic_patch() {
        let old_state = json!({ "counter": 1, "foo": true });
        let new_state = json!({ "counter": 3, "bar": "x" });

        let patch_one = diff(&old_state, &new_state);
        let patch_two = diff(&old_state, &new_state);

        assert_eq!(patch_one, patch_two);
        assert_eq!(patch_one.len(), 3);
        assert_eq!(
            patch_one,
            vec![
                PatchOp::Set {
                    path: "/bar".to_owned(),
                    value: json!("x")
                },
                PatchOp::Set {
                    path: "/counter".to_owned(),
                    value: json!(3)
                },
                PatchOp::Del {
                    path: "/foo".to_owned()
                }
            ]
        );
    }

    #[test]
    fn applying_patch_results_in_correct_state() {
        let mut state = json!({ "counter": 1, "foo": true });
        let target = json!({ "counter": 3, "bar": "x" });

        let patch = diff(&state, &target);
        apply_patch(&mut state, &patch).expect("patch apply should pass");

        assert_eq!(state, target);
    }

    #[test]
    fn patch_encoding_stable_across_codecs() {
        let patch = vec![
            PatchOp::Set {
                path: "/counter".to_owned(),
                value: json!(3),
            },
            PatchOp::Del {
                path: "/foo".to_owned(),
            },
        ];

        let msg = Message {
            v: PROTOCOL_VERSION,
            t: "state.patch".to_owned(),
            rid: None,
            room: Some("counter-room".to_owned()),
            p: Some(serde_json::to_value(&patch).expect("patch to value")),
        };

        let json_codec = JsonCodec;
        let msgpack_codec = MsgpackCodec;

        let json_decoded = json_codec
            .decode(&json_codec.encode(&msg))
            .expect("json decode must pass");
        let msgpack_decoded = msgpack_codec
            .decode(&msgpack_codec.encode(&msg))
            .expect("msgpack decode must pass");

        let json_patch: Vec<PatchOp> =
            serde_json::from_value(json_decoded.p.expect("payload")).expect("json payload parse");
        let msgpack_patch: Vec<PatchOp> =
            serde_json::from_value(msgpack_decoded.p.expect("payload"))
                .expect("msgpack payload parse");

        assert_eq!(json_patch, patch);
        assert_eq!(msgpack_patch, patch);
    }

    #[test]
    fn checksum_is_stable_for_equivalent_objects() {
        let left = json!({
            "b": 2,
            "a": {
                "z": true,
                "k": [3, 2, 1]
            }
        });
        let right = json!({
            "a": {
                "k": [3, 2, 1],
                "z": true
            },
            "b": 2
        });

        let left_checksum = state_checksum(&left);
        let right_checksum = state_checksum(&right);
        assert_eq!(left_checksum, right_checksum);
    }

    #[test]
    fn checksum_changes_when_state_changes() {
        let initial = json!({ "counter": 1 });
        let changed = json!({ "counter": 2 });

        assert_ne!(state_checksum(&initial), state_checksum(&changed));
    }

    #[test]
    fn applying_patch_matches_target_checksum() {
        let mut state = json!({ "counter": 5, "foo": true });
        let target = json!({ "counter": 7, "bar": "x" });

        let patch = diff(&state, &target);
        apply_patch(&mut state, &patch).expect("patch apply should pass");

        assert_eq!(state_checksum(&state), state_checksum(&target));
    }
}
