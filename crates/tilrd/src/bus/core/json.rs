//! The proto3 canonical JSON mapping of the core's payloads (B9): a request with
//! `application/json` is answered in JSON, so scripts and `tilr bus call` need no
//! codec. Events and stream chunks of the core stay protobuf.
//!
//! Owner: `bus-core-panes`. The other namespaces of the core (`bus-core-services`,
//! `pane-profiles`) implement [`Json`] for their messages with the readers and
//! writers here and answer with [`decode`] and [`reply`].
//!
//! The mapping as written, with what the specification leaves to the
//! implementation: fields are written under their lowerCamelCase JSON names, and
//! read under that name or the proto name; every field is written, defaults too
//! (scripts look for `panes` in an empty list), except an unset `optional` field
//! or message; 64-bit integers are written as strings and read from strings or
//! numbers; `bytes` are base64 (read with or without padding, standard or URL
//! alphabet); enums are written by name and read by name or number; `null` is the
//! default; unknown fields are ignored (the repo's rule for protobuf, kept).

use bytes::Bytes;
use prost::Message;
use serde_json::{Map, Value, json};
use tilr_bus::pane;
use tilr_bus::{BusError, Reply};

use crate::bus::identity::CallCtx;

/// A message in the proto3 JSON mapping.
pub trait Json: Sized {
    fn to_json(&self) -> Value;
    /// `Err` holds what is wrong, without the method's name.
    fn from_json(value: &Value) -> Result<Self, String>;
}

/// The request of a call to `method`: JSON when the caller sent
/// `application/json`, else protobuf. A request that does not decode is
/// `bad_request`; an empty payload is the default message either way.
pub fn decode<T: Message + Default + Json>(ctx: &CallCtx, payload: &[u8]) -> Result<T, BusError> {
    if ctx.wants_json() {
        if payload.iter().all(u8::is_ascii_whitespace) {
            return T::from_json(&Value::Null)
                .map_err(|err| BusError::bad_request(format!("{}: {err}", ctx.method)));
        }
        let value: Value = serde_json::from_slice(payload)
            .map_err(|err| BusError::bad_request(format!("{}: {err}", ctx.method)))?;
        return T::from_json(&value)
            .map_err(|err| BusError::bad_request(format!("{}: {err}", ctx.method)));
    }
    T::decode(payload).map_err(|err| BusError::bad_request(format!("{}: {err}", ctx.method)))
}

/// The reply to a call: in the encoding of its request.
pub fn reply<T: Message + Json>(ctx: &CallCtx, message: &T) -> Reply {
    if ctx.wants_json() {
        Reply::new(message.to_json().to_string()).with_content_type(tilr_bus::CONTENT_TYPE_JSON)
    } else {
        Reply::encode(message)
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// The fields of a JSON object, read by JSON name or proto name.
pub struct Fields<'a>(Option<&'a Map<String, Value>>);

impl<'a> Fields<'a> {
    /// `null` is an empty message; anything else not an object is an error.
    pub fn of(value: &'a Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(Self(None)),
            Value::Object(map) => Ok(Self(Some(map))),
            other => Err(format!("expected an object, got {}", kind(other))),
        }
    }

    /// The field under its JSON name or its proto name; `None` when absent or null.
    pub fn get(&self, json_name: &str, proto_name: &str) -> Option<&'a Value> {
        let map = self.0?;
        map.get(json_name)
            .or_else(|| map.get(proto_name))
            .filter(|value| !value.is_null())
    }

    pub fn string(&self, json_name: &str, proto_name: &str) -> Result<String, String> {
        match self.get(json_name, proto_name) {
            None => Ok(String::new()),
            Some(Value::String(text)) => Ok(text.clone()),
            Some(other) => Err(field_error(json_name, "a string", other)),
        }
    }

    pub fn strings(&self, json_name: &str, proto_name: &str) -> Result<Vec<String>, String> {
        self.list(json_name, proto_name, |value| match value {
            Value::String(text) => Ok(text.clone()),
            other => Err(field_error(json_name, "a string", other)),
        })
    }

    pub fn bool(&self, json_name: &str, proto_name: &str) -> Result<bool, String> {
        match self.get(json_name, proto_name) {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(other) => Err(field_error(json_name, "true or false", other)),
        }
    }

    pub fn u32(&self, json_name: &str, proto_name: &str) -> Result<u32, String> {
        Ok(self.opt_u32(json_name, proto_name)?.unwrap_or(0))
    }

    pub fn opt_u32(&self, json_name: &str, proto_name: &str) -> Result<Option<u32>, String> {
        self.get(json_name, proto_name)
            .map(|value| {
                integer(value)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| field_error(json_name, "a uint32", value))
            })
            .transpose()
    }

    pub fn i32(&self, json_name: &str, proto_name: &str) -> Result<i32, String> {
        self.get(json_name, proto_name)
            .map(|value| {
                integer(value)
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or_else(|| field_error(json_name, "an int32", value))
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn u64(&self, json_name: &str, proto_name: &str) -> Result<u64, String> {
        Ok(self.opt_u64(json_name, proto_name)?.unwrap_or(0))
    }

    pub fn opt_u64(&self, json_name: &str, proto_name: &str) -> Result<Option<u64>, String> {
        self.get(json_name, proto_name)
            .map(|value| {
                integer(value)
                    .and_then(|n| u64::try_from(n).ok())
                    .ok_or_else(|| field_error(json_name, "a uint64", value))
            })
            .transpose()
    }

    pub fn bytes(&self, json_name: &str, proto_name: &str) -> Result<Bytes, String> {
        match self.get(json_name, proto_name) {
            None => Ok(Bytes::new()),
            Some(Value::String(text)) => base64_decode(text)
                .map(Bytes::from)
                .ok_or_else(|| format!("{json_name}: not base64")),
            Some(other) => Err(field_error(json_name, "a base64 string", other)),
        }
    }

    /// An enum field: its name (`from_name`) or its number.
    pub fn enumeration(
        &self,
        json_name: &str,
        proto_name: &str,
        from_name: impl Fn(&str) -> Option<i32>,
    ) -> Result<i32, String> {
        match self.get(json_name, proto_name) {
            None => Ok(0),
            Some(Value::String(name)) => {
                from_name(name).ok_or_else(|| format!("{json_name}: no such value {name:?}"))
            }
            Some(value) => integer(value)
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| field_error(json_name, "an enum name or number", value)),
        }
    }

    /// A message field; `None` when absent.
    pub fn message<T: Json>(&self, json_name: &str, proto_name: &str) -> Result<Option<T>, String> {
        self.get(json_name, proto_name)
            .map(|value| T::from_json(value).map_err(|err| format!("{json_name}.{err}")))
            .transpose()
    }

    pub fn messages<T: Json>(&self, json_name: &str, proto_name: &str) -> Result<Vec<T>, String> {
        self.list(json_name, proto_name, |value| {
            T::from_json(value).map_err(|err| format!("{json_name}.{err}"))
        })
    }

    fn list<T>(
        &self,
        json_name: &str,
        proto_name: &str,
        item: impl Fn(&Value) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        match self.get(json_name, proto_name) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items.iter().map(item).collect(),
            Some(other) => Err(field_error(json_name, "an array", other)),
        }
    }
}

/// An integer as JSON gives it: a number without a fraction, or a string of one.
fn integer(value: &Value) -> Option<i128> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from))
            .or_else(|| {
                // 3.0 is an integer in JSON's sense.
                number
                    .as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 1e19)
                    .map(|f| f as i128)
            }),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn field_error(name: &str, expected: &str, got: &Value) -> String {
    format!("{name}: expected {expected}, got {}", kind(got))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// A 64-bit integer as proto3 JSON writes it: a string.
pub fn int64(value: impl std::fmt::Display) -> Value {
    Value::String(value.to_string())
}

/// `bytes` as proto3 JSON writes them: standard base64 with padding.
pub fn bytes(data: &[u8]) -> Value {
    Value::String(base64_encode(data))
}

/// A list of messages.
pub fn messages<T: Json>(items: &[T]) -> Value {
    Value::Array(items.iter().map(Json::to_json).collect())
}

/// An enum value by name; a number this build does not know stays a number.
pub fn enumeration(value: i32, name: Option<&str>) -> Value {
    name.map_or_else(|| json!(value), |name| json!(name))
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard or URL alphabet, padded or not; `None` if it is not base64.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut n = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        n = n << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    // A lone sixth of a byte left over is not base64.
    (bits < 6).then_some(out)
}

// ---------------------------------------------------------------------------
// tilr.pane.v1
// ---------------------------------------------------------------------------

impl Json for pane::Viewer {
    fn to_json(&self) -> Value {
        json!({
            "clientId": self.client_id,
            "clientName": self.client_name,
            "address": self.address,
            "identity": self.identity,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            client_id: f.string("clientId", "client_id")?,
            client_name: f.string("clientName", "client_name")?,
            address: f.string("address", "address")?,
            identity: f.string("identity", "identity")?,
        })
    }
}

impl Json for pane::PaneInfo {
    fn to_json(&self) -> Value {
        json!({
            "paneId": self.pane_id,
            "cols": self.cols,
            "rows": self.rows,
            "command": self.command,
            "title": self.title,
            "cwd": self.cwd,
            "viewers": self.viewers,
            "createdAtMs": int64(self.created_at_ms),
            "pid": self.pid,
            "viewedBy": messages(&self.viewed_by),
            "usedAtMs": int64(self.used_at_ms),
            "createdBy": self.created_by,
            "sandboxed": self.sandboxed,
            "profile": self.profile,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            cols: f.u32("cols", "cols")?,
            rows: f.u32("rows", "rows")?,
            command: f.strings("command", "command")?,
            title: f.string("title", "title")?,
            cwd: f.string("cwd", "cwd")?,
            viewers: f.u32("viewers", "viewers")?,
            created_at_ms: f.u64("createdAtMs", "created_at_ms")?,
            pid: f.u32("pid", "pid")?,
            viewed_by: f.messages("viewedBy", "viewed_by")?,
            used_at_ms: f.u64("usedAtMs", "used_at_ms")?,
            created_by: f.string("createdBy", "created_by")?,
            sandboxed: f.bool("sandboxed", "sandboxed")?,
            profile: f.string("profile", "profile")?,
        })
    }
}

impl Json for pane::CreateRequest {
    fn to_json(&self) -> Value {
        json!({
            "cols": self.cols,
            "rows": self.rows,
            "command": self.command,
            "cwd": self.cwd,
            "profile": self.profile,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            cols: f.u32("cols", "cols")?,
            rows: f.u32("rows", "rows")?,
            command: f.strings("command", "command")?,
            cwd: f.string("cwd", "cwd")?,
            profile: f.string("profile", "profile")?,
        })
    }
}

impl Json for pane::ListRequest {
    fn to_json(&self) -> Value {
        json!({})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Fields::of(value)?;
        Ok(Self {})
    }
}

impl Json for pane::ListReply {
    fn to_json(&self) -> Value {
        json!({ "panes": messages(&self.panes) })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            panes: f.messages("panes", "panes")?,
        })
    }
}

impl Json for pane::CloseRequest {
    fn to_json(&self) -> Value {
        json!({ "paneId": self.pane_id })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
        })
    }
}

impl Json for pane::CloseReply {
    fn to_json(&self) -> Value {
        json!({ "exitCode": self.exit_code })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            exit_code: f.i32("exitCode", "exit_code")?,
        })
    }
}

impl Json for pane::OutputRequest {
    fn to_json(&self) -> Value {
        json!({
            "paneId": self.pane_id,
            "cellWidthPx": self.cell_width_px,
            "cellHeightPx": self.cell_height_px,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            cell_width_px: f.u32("cellWidthPx", "cell_width_px")?,
            cell_height_px: f.u32("cellHeightPx", "cell_height_px")?,
        })
    }
}

impl Json for pane::PaneExited {
    fn to_json(&self) -> Value {
        json!({ "paneId": self.pane_id, "exitCode": self.exit_code })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            exit_code: f.i32("exitCode", "exit_code")?,
        })
    }
}

impl Json for pane::ReadRequest {
    fn to_json(&self) -> Value {
        let mut object = json!({
            "paneId": self.pane_id,
            "historyLines": self.history_lines,
        });
        if let Some(since) = self.since_seq {
            object["sinceSeq"] = int64(since);
        }
        if let Some(wait) = self.wait_ms {
            object["waitMs"] = json!(wait);
        }
        object
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            since_seq: f.opt_u64("sinceSeq", "since_seq")?,
            wait_ms: f.opt_u32("waitMs", "wait_ms")?,
            history_lines: f.u32("historyLines", "history_lines")?,
        })
    }
}

impl Json for pane::Cursor {
    fn to_json(&self) -> Value {
        json!({
            "visible": self.visible,
            "x": self.x,
            "y": self.y,
            "shape": enumeration(
                self.shape,
                pane::CursorShape::try_from(self.shape).ok().map(|s| s.as_str_name()),
            ),
            "blinking": self.blinking,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            visible: f.bool("visible", "visible")?,
            x: f.u32("x", "x")?,
            y: f.u32("y", "y")?,
            shape: f.enumeration("shape", "shape", |name| {
                pane::CursorShape::from_str_name(name).map(|s| s as i32)
            })?,
            blinking: f.bool("blinking", "blinking")?,
        })
    }
}

impl Json for pane::ReadReply {
    fn to_json(&self) -> Value {
        let mut object = json!({
            "seq": int64(self.seq),
            "cols": self.cols,
            "rows": self.rows,
            "lines": self.lines,
            "title": self.title,
            "cwd": self.cwd,
            "alive": self.alive,
            "history": self.history,
        });
        if let Some(cursor) = &self.cursor {
            object["cursor"] = cursor.to_json();
        }
        if let Some(code) = self.exit_code {
            object["exitCode"] = json!(code);
        }
        object
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            seq: f.u64("seq", "seq")?,
            cols: f.u32("cols", "cols")?,
            rows: f.u32("rows", "rows")?,
            lines: f.strings("lines", "lines")?,
            cursor: f.message("cursor", "cursor")?,
            title: f.string("title", "title")?,
            cwd: f.string("cwd", "cwd")?,
            alive: f.bool("alive", "alive")?,
            exit_code: f
                .get("exitCode", "exit_code")
                .map(|_| f.i32("exitCode", "exit_code"))
                .transpose()?,
            history: f.strings("history", "history")?,
        })
    }
}

impl Json for pane::KeyInput {
    fn to_json(&self) -> Value {
        json!({
            "action": enumeration(
                self.action,
                pane::KeyAction::try_from(self.action).ok().map(|a| a.as_str_name()),
            ),
            "mods": self.mods,
            "named": enumeration(
                self.named,
                pane::NamedKey::try_from(self.named).ok().map(|k| k.as_str_name()),
            ),
            "codepoint": self.codepoint,
            "text": self.text,
            "code": self.code,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            action: f.enumeration("action", "action", |name| {
                pane::KeyAction::from_str_name(name).map(|a| a as i32)
            })?,
            mods: f.u32("mods", "mods")?,
            named: f.enumeration("named", "named", |name| {
                pane::NamedKey::from_str_name(name).map(|k| k as i32)
            })?,
            codepoint: f.u32("codepoint", "codepoint")?,
            text: f.string("text", "text")?,
            code: f.string("code", "code")?,
        })
    }
}

impl Json for pane::MouseInput {
    fn to_json(&self) -> Value {
        json!({
            "action": enumeration(
                self.action,
                pane::MouseAction::try_from(self.action).ok().map(|a| a.as_str_name()),
            ),
            "button": enumeration(
                self.button,
                pane::MouseButton::try_from(self.button).ok().map(|b| b.as_str_name()),
            ),
            "mods": self.mods,
            "x": self.x,
            "y": self.y,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            action: f.enumeration("action", "action", |name| {
                pane::MouseAction::from_str_name(name).map(|a| a as i32)
            })?,
            button: f.enumeration("button", "button", |name| {
                pane::MouseButton::from_str_name(name).map(|b| b as i32)
            })?,
            mods: f.u32("mods", "mods")?,
            x: f.u32("x", "x")?,
            y: f.u32("y", "y")?,
        })
    }
}

impl Json for pane::Paste {
    fn to_json(&self) -> Value {
        json!({ "text": self.text })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            text: f.string("text", "text")?,
        })
    }
}

impl Json for pane::RawInput {
    fn to_json(&self) -> Value {
        json!({ "data": bytes(&self.data) })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            data: f.bytes("data", "data")?,
        })
    }
}

impl Json for pane::Scroll {
    fn to_json(&self) -> Value {
        json!({ "delta": self.delta, "toBottom": self.to_bottom })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            delta: f.i32("delta", "delta")?,
            to_bottom: f.bool("toBottom", "to_bottom")?,
        })
    }
}

impl Json for pane::ClearPane {
    fn to_json(&self) -> Value {
        json!({ "history": self.history })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            history: f.bool("history", "history")?,
        })
    }
}

impl Json for pane::InputRequest {
    fn to_json(&self) -> Value {
        use pane::input_request::Input;
        let mut object = json!({
            "paneId": self.pane_id,
            "seq": int64(self.seq),
            "inputSeq": int64(self.input_seq),
        });
        let (name, input) = match &self.input {
            Some(Input::Key(key)) => ("key", key.to_json()),
            Some(Input::Mouse(mouse)) => ("mouse", mouse.to_json()),
            Some(Input::Paste(paste)) => ("paste", paste.to_json()),
            Some(Input::Raw(raw)) => ("raw", raw.to_json()),
            Some(Input::Scroll(scroll)) => ("scroll", scroll.to_json()),
            Some(Input::Clear(clear)) => ("clear", clear.to_json()),
            None => return object,
        };
        object[name] = input;
        object
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        use pane::input_request::Input;
        let f = Fields::of(value)?;
        let inputs = [
            f.message("key", "key")?.map(Input::Key),
            f.message("mouse", "mouse")?.map(Input::Mouse),
            f.message("paste", "paste")?.map(Input::Paste),
            f.message("raw", "raw")?.map(Input::Raw),
            f.message("scroll", "scroll")?.map(Input::Scroll),
            f.message("clear", "clear")?.map(Input::Clear),
        ];
        let mut inputs = inputs.into_iter().flatten();
        let input = inputs.next();
        if inputs.next().is_some() {
            return Err("more than one of key, mouse, paste, raw, scroll, clear".into());
        }
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            input,
            seq: f.u64("seq", "seq")?,
            input_seq: f.u64("inputSeq", "input_seq")?,
        })
    }
}

impl Json for pane::ResizeRequest {
    fn to_json(&self) -> Value {
        json!({
            "paneId": self.pane_id,
            "cols": self.cols,
            "rows": self.rows,
            "cellWidthPx": self.cell_width_px,
            "cellHeightPx": self.cell_height_px,
            "seq": int64(self.seq),
            "inputSeq": int64(self.input_seq),
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            cols: f.u32("cols", "cols")?,
            rows: f.u32("rows", "rows")?,
            cell_width_px: f.u32("cellWidthPx", "cell_width_px")?,
            cell_height_px: f.u32("cellHeightPx", "cell_height_px")?,
            seq: f.u64("seq", "seq")?,
            input_seq: f.u64("inputSeq", "input_seq")?,
        })
    }
}

impl Json for pane::InputAck {
    fn to_json(&self) -> Value {
        json!({ "paneId": self.pane_id, "seq": int64(self.seq) })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            pane_id: f.string("paneId", "pane_id")?,
            seq: f.u64("seq", "seq")?,
        })
    }
}

impl Json for pane::AppliedRequest {
    fn to_json(&self) -> Value {
        json!({})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Fields::of(value)?;
        Ok(Self {})
    }
}

impl Json for pane::AppliedReply {
    fn to_json(&self) -> Value {
        json!({ "applied": messages(&self.applied) })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let f = Fields::of(value)?;
        Ok(Self {
            applied: f.messages("applied", "applied")?,
        })
    }
}

impl Json for pane::InputFinalizeRequest {
    fn to_json(&self) -> Value {
        json!({})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Fields::of(value)?;
        Ok(Self {})
    }
}

impl Json for pane::InputFinalizeReply {
    fn to_json(&self) -> Value {
        json!({})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Fields::of(value)?;
        Ok(Self {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_back_what_it_writes() {
        for data in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0xff, 0, 0xfe],
        ] {
            let text = base64_encode(data);
            assert_eq!(base64_decode(&text).as_deref(), Some(data), "{text}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        // Unpadded and URL-safe read too; a stray character does not.
        assert_eq!(base64_decode("Zm8").as_deref(), Some(&b"fo"[..]));
        assert_eq!(base64_decode("-_8").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(base64_decode("Zm9v!"), None);
        assert_eq!(base64_decode("Z"), None);
    }

    #[test]
    fn pane_messages_map_to_proto3_json_and_back() {
        let info = pane::PaneInfo {
            pane_id: "p".into(),
            cols: 80,
            rows: 24,
            command: vec!["sh".into()],
            created_at_ms: 1_700_000_000_000,
            created_by: "owner".into(),
            viewed_by: vec![pane::Viewer {
                client_name: "c".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let value = info.to_json();
        // 64-bit integers are strings; defaults are written; names are lowerCamelCase.
        assert_eq!(value["createdAtMs"], "1700000000000");
        assert_eq!(value["title"], "");
        assert_eq!(value["viewedBy"][0]["clientName"], "c");
        assert_eq!(pane::PaneInfo::from_json(&value).unwrap(), info);

        // Requests read under either name, numbers for 64-bit fields too, enums
        // by name or number, unknown fields ignored.
        let input = pane::InputRequest::from_json(&json!({
            "pane_id": "p",
            "inputSeq": 7,
            "seq": "4294967297",
            "key": {"action": "KEY_ACTION_RELEASE", "named": 1, "text": "\r"},
            "later": true,
        }))
        .unwrap();
        assert_eq!(input.pane_id, "p");
        assert_eq!((input.seq, input.input_seq), (4_294_967_297, 7));
        let Some(pane::input_request::Input::Key(key)) = &input.input else {
            panic!("{input:?}");
        };
        assert_eq!(key.action, pane::KeyAction::Release as i32);
        assert_eq!(key.named, pane::NamedKey::Enter as i32);
        assert_eq!(
            pane::InputRequest::from_json(&input.to_json()).unwrap(),
            input
        );

        let raw = pane::InputRequest::from_json(&json!({"raw": {"data": "AP8="}})).unwrap();
        assert_eq!(
            raw.input,
            Some(pane::input_request::Input::Raw(pane::RawInput {
                data: Bytes::from_static(&[0, 0xff]),
            }))
        );

        assert!(pane::InputRequest::from_json(&json!({"raw": {}, "paste": {}})).is_err());
        assert!(pane::CreateRequest::from_json(&json!({"cols": -1})).is_err());
        assert!(pane::CreateRequest::from_json(&json!([])).is_err());
        assert_eq!(
            pane::CreateRequest::from_json(&Value::Null).unwrap(),
            pane::CreateRequest::default()
        );

        // Optional fields: absent stays absent.
        let read = pane::ReadRequest::from_json(&json!({"paneId": "p"})).unwrap();
        assert_eq!((read.since_seq, read.wait_ms), (None, None));
        let read = pane::ReadRequest::from_json(&json!({"sinceSeq": "5", "waitMs": 0})).unwrap();
        assert_eq!((read.since_seq, read.wait_ms), (Some(5), Some(0)));
        let reply = pane::ReadReply {
            exit_code: Some(3),
            ..Default::default()
        };
        assert_eq!(reply.to_json()["exitCode"], 3);
        assert!(
            pane::ReadReply::default()
                .to_json()
                .get("exitCode")
                .is_none()
        );
        assert_eq!(pane::ReadReply::from_json(&reply.to_json()).unwrap(), reply);
    }
}
