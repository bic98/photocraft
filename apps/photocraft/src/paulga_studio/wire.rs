//! The messages PAULGA Photo and PAULGA Studio exchange: JSON envelopes `{type, payload,
//! timestamp}` sent as WebSocket text. Requests and their answers share `payload.requestId`.
//! Studio's side is `src-tauri/src/cad/session.rs` in its repository; it names every program's
//! messages `cad:…`, this one's too.

use photocraft_ui_egui::studio::Document;
use serde_json::{Value, json};

/// Studio closes a connection that sends a longer message.
pub(super) const MAX_MESSAGE_BYTES: usize = 50 * 1024 * 1024;
/// Room for the envelope around a result that is close to the limit.
const HEADROOM_BYTES: usize = 64 * 1024;

pub(super) fn envelope(kind: &str, payload: Value) -> String {
    let timestamp = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    json!({ "type": kind, "payload": payload, "timestamp": timestamp }).to_string()
}

/// A request id or tool name Studio may send: a letter or digit, then up to 127 of
/// letters, digits and `. _ : -`.
pub(super) fn identifier(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let mut chars = text.chars();
    let first = chars.next()?;
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
    (first.is_ascii_alphanumeric() && rest_ok && text.len() <= 128).then_some(text)
}

/// The whole of what Studio is told about this program and its document. `contextComplete` makes
/// it authoritative: a field left out (the path of an unsaved document) clears Studio's copy.
pub(super) fn status(document: &Document) -> Value {
    let mut payload = json!({
        "status": if document.busy { "busy" } else { "idle" },
        "contextComplete": true,
        "supportsDocumentFence": true,
        "pluginVersion": env!("CARGO_PKG_VERSION"),
        "cadVersion": photocraft_engine::build_info::long_version(),
        "runtimeDocumentId": document.runtime_id,
    });
    if let Some(name) = &document.name {
        payload["documentName"] = json!(name);
    }
    if let Some(path) = &document.path {
        payload["documentPath"] = json!(path);
        payload["modelPath"] = json!(path);
    }
    payload
}

/// The answer to `cad:execute-tool`. A result too large to send becomes a failure for the same
/// request, so Studio gets an answer instead of waiting for one.
pub(super) fn tool_result(request_id: &str, tool: &str, outcome: Result<Value, String>, elapsed_ms: u128) -> String {
    let base = |success: bool| json!({ "requestId": request_id, "toolName": tool, "success": success, "executionTimeMs": u64::try_from(elapsed_ms).unwrap_or(u64::MAX) });
    let failed = |error: String| {
        let mut payload = base(false);
        payload["error"] = json!(error);
        envelope("cad:tool-result", payload)
    };
    match outcome {
        Err(error) => failed(error),
        Ok(result) => {
            let mut payload = base(true);
            payload["result"] = result;
            let text = envelope("cad:tool-result", payload);
            if text.len() <= MAX_MESSAGE_BYTES - HEADROOM_BYTES {
                text
            } else {
                failed(format!("RESULT_TOO_LARGE: the result is {:.1} MB; the limit is 50 MB. Ask for less (a smaller page or picture)", text.len() as f64 / 1048576.0))
            }
        }
    }
}

/// The answer to `cad:request-data`, which this program does not serve: its reads are tools.
pub(super) fn data_unsupported(request_id: &Value) -> String {
    let error = json!({ "code": "UNSUPPORTED", "message": "PAULGA Photo answers through tools; get-capabilities lists them." });
    envelope("cad:data", json!({ "requestId": request_id, "error": error, "data": { "success": false, "error": error } }))
}

/// Studio's only `disconnect` reason: it is exiting, so there is nothing to reconnect to.
pub(super) fn shutting_down(payload: &Value) -> bool {
    ["reason", "message"].iter().filter_map(|key| payload.get(*key).and_then(Value::as_str)).any(|text| text.to_ascii_lowercase().contains("shutting down"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(text: &str) -> Value {
        serde_json::from_str::<Value>(text).unwrap()["payload"].clone()
    }

    #[test]
    fn an_unsaved_document_sends_no_path_so_studio_forgets_the_last_one() {
        let saved = Document { name: Some("Plan.psd".into()), path: Some("C:/work/Plan.psd".into()), runtime_id: "7-1:2".into(), busy: false };
        let s = status(&saved);
        assert_eq!(s["documentPath"], "C:/work/Plan.psd");
        assert_eq!(s["contextComplete"], true);
        assert_eq!(s["runtimeDocumentId"], "7-1:2");
        let unsaved = Document { path: None, busy: true, ..saved };
        let s = status(&unsaved);
        assert!(s.get("documentPath").is_none() && s.get("modelPath").is_none());
        assert_eq!(s["status"], "busy");
    }

    #[test]
    fn a_result_studio_would_drop_is_answered_as_a_failure_of_the_same_request() {
        let ok = payload(&tool_result("r-1", "get-context", Ok(json!({ "a": 1 })), 12));
        assert_eq!((ok["success"].clone(), ok["result"]["a"].clone(), ok["requestId"].clone()), (json!(true), json!(1), json!("r-1")));
        let huge = Value::String("x".repeat(MAX_MESSAGE_BYTES));
        let sent = tool_result("r-2", "capture-view", Ok(huge), 0);
        assert!(sent.len() < 4096);
        let failed = payload(&sent);
        assert_eq!(failed["success"], false);
        assert_eq!(failed["requestId"], "r-2");
        assert!(failed["error"].as_str().unwrap().starts_with("RESULT_TOO_LARGE"));
    }

    #[test]
    fn only_plain_identifiers_name_a_request_or_a_tool() {
        for good in ["get-context", "3f2a:9", "a.b_c-d"] {
            assert_eq!(identifier(&json!(good)), Some(good));
        }
        let long = "a".repeat(129);
        for bad in [json!(""), json!("-x"), json!("a b"), json!("a/b"), json!(long), json!(7), Value::Null] {
            assert_eq!(identifier(&bad), None, "{bad}");
        }
    }

    #[test]
    fn studio_exiting_is_told_apart_from_a_dropped_connection() {
        assert!(shutting_down(&json!({ "reason": "Desktop app shutting down" })));
        assert!(!shutting_down(&json!({ "reason": "replaced by a newer connection" })));
        assert!(!shutting_down(&json!({})));
    }
}
