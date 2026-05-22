//! Lenient `serde` deserializers for the MCP / REST request boundary.
//!
//! Some MCP clients serialize *every* tool-call argument as a JSON string,
//! ignoring the `integer` / `array` / `boolean` types advertised in the tool
//! schema (tracked upstream as anthropics/claude-code#24599). Without coercion
//! the server rejects such calls with errors like
//! `-32602 invalid type: string "3", expected usize`, or silently drops
//! `categories`. These helpers accept both the native JSON type and its
//! stringified form so a misbehaving client degrades gracefully.
//!
//! Apply them with `#[serde(deserialize_with = "...")]`. Optional fields must
//! also carry `#[serde(default)]` so an absent field still resolves to `None`.

use std::fmt::Display;
use std::str::FromStr;

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer, Error};
use serde_json::Value;

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn number_from_value<T>(value: Value) -> Result<T, String>
where
    T: FromStr,
    T::Err: Display,
{
    match value {
        Value::Number(n) => n
            .to_string()
            .parse::<T>()
            .map_err(|e| format!("invalid number `{n}`: {e}")),
        Value::String(s) => {
            tracing::debug!(raw = %s, "coercing stringified number param");
            s.trim()
                .parse::<T>()
                .map_err(|e| format!("invalid numeric string `{s}`: {e}"))
        }
        other => Err(format!(
            "expected a number or numeric string, got {}",
            kind_of(&other)
        )),
    }
}

/// Deserialize a required numeric field from a JSON number or numeric string.
pub fn number<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let value = Value::deserialize(deserializer)?;
    number_from_value(value).map_err(D::Error::custom)
}

/// Deserialize an optional numeric field. An absent field, `null`, or an empty
/// string all resolve to `None`. Requires `#[serde(default)]` on the field.
pub fn opt_number<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(value) => number_from_value(value).map(Some).map_err(D::Error::custom),
    }
}

fn bool_from_value(value: Value) -> Result<bool, String> {
    match value {
        Value::Bool(b) => Ok(b),
        Value::String(s) => {
            tracing::debug!(raw = %s, "coercing stringified boolean param");
            match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" | "on" => Ok(true),
                "false" | "0" | "no" | "n" | "off" => Ok(false),
                other => Err(format!("invalid boolean string `{other}`")),
            }
        }
        Value::Number(n) => Ok(n.as_i64().map(|i| i != 0).unwrap_or(false)),
        other => Err(format!("expected a boolean, got {}", kind_of(&other))),
    }
}

/// Deserialize a required boolean field from a JSON bool or string.
pub fn boolean<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    bool_from_value(value).map_err(D::Error::custom)
}

/// Deserialize an optional boolean field. An absent field or `null` resolves to
/// `None`. Requires `#[serde(default)]` on the field.
pub fn opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(value) => bool_from_value(value).map(Some).map_err(D::Error::custom),
    }
}

fn string_vec_from_value(value: Value) -> Result<Vec<String>, String> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|item| match item {
                Value::String(s) => Ok(s),
                Value::Number(n) => Ok(n.to_string()),
                Value::Bool(b) => Ok(b.to_string()),
                other => Err(format!(
                    "array element is not a string: got {}",
                    kind_of(&other)
                )),
            })
            .collect(),
        Value::String(s) => {
            tracing::debug!(raw = %s, "coercing stringified array param");
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(Vec::new())
            } else if trimmed.starts_with('[') {
                serde_json::from_str::<Vec<String>>(trimmed)
                    .map_err(|e| format!("invalid JSON array string `{s}`: {e}"))
            } else {
                // A bare, non-JSON string is treated as a single-element list.
                Ok(vec![s])
            }
        }
        Value::Null => Ok(Vec::new()),
        other => Err(format!(
            "expected an array of strings, got {}",
            kind_of(&other)
        )),
    }
}

/// Deserialize a required `Vec<String>` from an array or a JSON-array string.
/// A bare non-JSON string becomes a single-element list.
pub fn string_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    string_vec_from_value(value).map_err(D::Error::custom)
}

/// Deserialize an optional `Vec<String>`. An absent field or `null` resolves to
/// `None`. Requires `#[serde(default)]` on the field.
pub fn opt_string_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => Ok(None),
        Some(value) => string_vec_from_value(value)
            .map(Some)
            .map_err(D::Error::custom),
    }
}

/// Deserialize a required `Vec<T>` of structs from an array or a JSON-array
/// string. Nested fields keep their own `deserialize_with` coercion.
pub fn json_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Array(_) => serde_json::from_value(value).map_err(D::Error::custom),
        Value::String(s) => {
            tracing::debug!(raw = %s, "coercing stringified array-of-objects param");
            serde_json::from_str(s.trim()).map_err(D::Error::custom)
        }
        Value::Null => Ok(Vec::new()),
        other => Err(D::Error::custom(format!(
            "expected an array, got {}",
            kind_of(&other)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct ReqNum {
        #[serde(deserialize_with = "number")]
        n: u32,
    }

    #[derive(Deserialize)]
    struct OptNum {
        #[serde(default, deserialize_with = "opt_number")]
        n: Option<usize>,
    }

    #[derive(Deserialize)]
    struct ReqFloat {
        #[serde(deserialize_with = "number")]
        f: f32,
    }

    #[derive(Deserialize)]
    struct OptFlag {
        #[serde(default, deserialize_with = "opt_bool")]
        b: Option<bool>,
    }

    #[derive(Deserialize)]
    struct ReqFlag {
        #[serde(default, deserialize_with = "boolean")]
        b: bool,
    }

    #[derive(Deserialize)]
    struct OptList {
        #[serde(default, deserialize_with = "opt_string_vec")]
        v: Option<Vec<String>>,
    }

    #[derive(Deserialize)]
    struct ReqList {
        #[serde(deserialize_with = "string_vec")]
        v: Vec<String>,
    }

    #[derive(Deserialize)]
    struct Inner {
        #[serde(deserialize_with = "number")]
        k: u32,
    }

    #[derive(Deserialize)]
    struct ObjList {
        #[serde(deserialize_with = "json_vec")]
        items: Vec<Inner>,
    }

    fn parse<T: DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).expect("should deserialize")
    }

    #[test]
    fn number_accepts_native_and_string() {
        assert_eq!(parse::<ReqNum>(r#"{"n":3}"#).n, 3);
        assert_eq!(parse::<ReqNum>(r#"{"n":"3"}"#).n, 3);
        assert_eq!(parse::<ReqNum>(r#"{"n":" 7 "}"#).n, 7);
    }

    #[test]
    fn number_rejects_garbage() {
        assert!(serde_json::from_str::<ReqNum>(r#"{"n":"abc"}"#).is_err());
        assert!(serde_json::from_str::<ReqNum>(r#"{"n":true}"#).is_err());
    }

    #[test]
    fn opt_number_handles_absent_null_and_empty() {
        assert_eq!(parse::<OptNum>(r#"{}"#).n, None);
        assert_eq!(parse::<OptNum>(r#"{"n":null}"#).n, None);
        assert_eq!(parse::<OptNum>(r#"{"n":""}"#).n, None);
        assert_eq!(parse::<OptNum>(r#"{"n":5}"#).n, Some(5));
        assert_eq!(parse::<OptNum>(r#"{"n":"5"}"#).n, Some(5));
    }

    #[test]
    fn float_accepts_int_and_string() {
        assert_eq!(parse::<ReqFloat>(r#"{"f":1}"#).f, 1.0);
        assert_eq!(parse::<ReqFloat>(r#"{"f":"0.5"}"#).f, 0.5);
        assert_eq!(parse::<ReqFloat>(r#"{"f":0.7}"#).f, 0.7);
    }

    #[test]
    fn bool_accepts_native_and_string() {
        assert!(parse::<ReqFlag>(r#"{"b":true}"#).b);
        assert!(parse::<ReqFlag>(r#"{"b":"true"}"#).b);
        assert!(!parse::<ReqFlag>(r#"{"b":"false"}"#).b);
        assert!(parse::<ReqFlag>(r#"{"b":"1"}"#).b);
        assert_eq!(parse::<OptFlag>(r#"{}"#).b, None);
        assert_eq!(parse::<OptFlag>(r#"{"b":"yes"}"#).b, Some(true));
    }

    #[test]
    fn string_vec_accepts_array_and_json_string() {
        assert_eq!(parse::<ReqList>(r#"{"v":["a","b"]}"#).v, vec!["a", "b"]);
        assert_eq!(
            parse::<ReqList>(r#"{"v":"[\"a\",\"b\"]"}"#).v,
            vec!["a", "b"]
        );
        assert_eq!(parse::<ReqList>(r#"{"v":"solo"}"#).v, vec!["solo"]);
        assert!(parse::<ReqList>(r#"{"v":""}"#).v.is_empty());
    }

    #[test]
    fn opt_string_vec_handles_absent() {
        assert_eq!(parse::<OptList>(r#"{}"#).v, None);
        assert_eq!(
            parse::<OptList>(r#"{"v":"[\"x\"]"}"#).v,
            Some(vec!["x".to_string()])
        );
    }

    #[test]
    fn json_vec_accepts_array_and_string_with_nested_coercion() {
        let from_array = parse::<ObjList>(r#"{"items":[{"k":"1"},{"k":2}]}"#);
        assert_eq!(from_array.items.len(), 2);
        assert_eq!(from_array.items[0].k, 1);
        assert_eq!(from_array.items[1].k, 2);

        let from_string = parse::<ObjList>(r#"{"items":"[{\"k\":\"9\"}]"}"#);
        assert_eq!(from_string.items[0].k, 9);
    }
}
