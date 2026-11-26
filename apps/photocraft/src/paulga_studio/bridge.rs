//! Finding the running PAULGA Studio through its connection file.
//!
//! Studio writes `%APPDATA%\@paulga\<app>\bridge.json` with a session token, its process id and,
//! for each program it talks to, whether it listens and where. A file counts only when Studio has
//! not marked it as exited and its PAULGA Photo listener is up on this computer. Only the port the
//! file names is ever tried: probing others would hand the token to whatever listens there.

use std::io::Read;
use std::path::{Path, PathBuf};

use photocraft_ui_egui::studio::PLATFORM;
use serde_json::Value;

const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A Studio that says it is listening for this program.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Bridge {
    pub(super) port: u16,
    pub(super) token: String,
    /// One run of Studio; a restart changes it.
    pub(super) instance: String,
    pub(super) desktop_pid: u32,
    pub(super) desktop_version: Option<String>,
    /// When Studio last wrote the file (ISO 8601, so text order is time order).
    updated: String,
}

impl std::fmt::Debug for Bridge {
    /// Never the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bridge(port {}, pid {})", self.port, self.desktop_pid)
    }
}

impl Bridge {
    /// What a refused sign-in is remembered by: the same token from the same run is not sent again.
    pub(super) fn session(&self) -> (String, String) {
        (self.instance.clone(), self.token.clone())
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The port of `ws://127.0.0.1:<port>`; any other host is not this computer.
fn loopback_port(endpoint: &str) -> Option<u16> {
    endpoint.strip_prefix("ws://127.0.0.1:")?.trim_end_matches('/').parse().ok().filter(|port| *port != 0)
}

/// Read one connection file's JSON. `None` when it is not a live offer for this program.
pub(super) fn parse(info: &Value) -> Option<Bridge> {
    if info.get("shutdownIntent").is_some_and(|v| !v.is_null() && v != "") {
        return None;
    }
    let host = info.get("status")?.get(PLATFORM)?;
    if host.get("listening") != Some(&Value::Bool(true)) {
        return None;
    }
    let port = host.get("endpoint").and_then(Value::as_str).and_then(loopback_port)?;
    let desktop_pid = info.get("desktopPid").and_then(Value::as_u64).and_then(|pid| u32::try_from(pid).ok()).filter(|pid| *pid != 0)?;
    Some(Bridge {
        port,
        token: text(info, "sessionToken")?,
        instance: text(info, "instanceId").or_else(|| text(info, "startedAt")).unwrap_or_else(|| desktop_pid.to_string()),
        desktop_pid,
        desktop_version: text(info, "desktopVersion"),
        updated: text(info, "updatedAt").unwrap_or_default(),
    })
}

fn read(path: &Path) -> Option<Value> {
    // Rust opens with delete sharing on Windows, so Studio can still replace the file.
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut raw = String::new();
    file.take(MAX_FILE_BYTES).read_to_string(&mut raw).ok()?;
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()
}

/// Every `<root>\@paulga\<app>\bridge.json`.
fn files(root: &Path) -> Vec<PathBuf> {
    let Ok(apps) = std::fs::read_dir(root.join("@paulga")) else { return Vec::new() };
    apps.filter_map(Result::ok).map(|app| app.path().join("bridge.json")).filter(|path| path.is_file()).collect()
}

/// Some Studio under `root` is up (it has not marked itself as exited) without listening for this
/// program: one from before PAULGA Photo was a program it connects to.
pub(super) fn studio_without_listener(root: &Path) -> bool {
    files(root).iter().filter_map(|path| read(path)).any(|info| {
        let exited = info.get("shutdownIntent").is_some_and(|v| !v.is_null() && v != "");
        !exited && info.get("status").is_some_and(Value::is_object) && parse(&info).is_none()
    })
}

/// The freshest live offer under `root` (`%APPDATA%`), skipping the Studio runs in `excluded`.
pub(super) fn find(root: &Path, excluded: &[String]) -> Option<Bridge> {
    files(root).iter().filter_map(|path| read(path)).filter_map(|info| parse(&info)).filter(|bridge| !excluded.contains(&bridge.instance)).max_by(|a, b| a.updated.cmp(&b.updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offer() -> Value {
        json!({
            "bridgeSchemaVersion": 3, "sessionToken": "secret", "instanceId": "run-1", "desktopPid": 4242,
            "desktopVersion": "0.1.80", "updatedAt": "2026-11-25T01:00:00.000Z",
            "ports": { "paulga-photo": 38370 },
            "status": { "paulga-photo": { "listening": true, "endpoint": "ws://127.0.0.1:38370" }, "blender": { "listening": true, "endpoint": "ws://127.0.0.1:38350" } }
        })
    }

    #[test]
    fn a_live_offer_names_the_port_studio_listens_on_for_this_program() {
        let bridge = parse(&offer()).unwrap();
        assert_eq!((bridge.port, bridge.desktop_pid, bridge.instance.as_str()), (38370, 4242, "run-1"));
        assert!(!format!("{bridge:?}").contains("secret"), "the token is never printed");
    }

    #[test]
    fn nothing_is_offered_by_a_studio_that_exited_is_not_listening_or_points_elsewhere() {
        let change = |edit: fn(&mut Value)| {
            let mut info = offer();
            edit(&mut info);
            parse(&info)
        };
        assert_eq!(change(|i| i["shutdownIntent"] = json!("user_exit")), None);
        assert_eq!(change(|i| i["status"]["paulga-photo"]["listening"] = json!(false)), None);
        assert_eq!(change(|i| i["status"]["paulga-photo"]["listening"] = json!("true")), None);
        assert_eq!(change(|i| i["status"]["paulga-photo"]["endpoint"] = json!("ws://203.0.113.9:38370")), None, "another computer");
        assert_eq!(change(|i| i["status"]["paulga-photo"]["endpoint"] = json!("ws://127.0.0.1:0")), None);
        assert_eq!(change(|i| i["sessionToken"] = json!("")), None);
        assert_eq!(change(|i| i["desktopPid"] = json!(0)), None);
        assert_eq!(change(|i| i["status"] = json!({ "blender": { "listening": true, "endpoint": "ws://127.0.0.1:38350" } })), None, "another program's port is never tried");
    }

    #[test]
    fn the_freshest_studio_is_chosen_and_a_refused_run_is_passed_over() {
        let root = std::env::temp_dir().join(format!("paulga-photo-bridge-{}-{:?}", std::process::id(), std::thread::current().id()));
        let write = |app: &str, instance: &str, updated: &str| {
            let dir = root.join("@paulga").join(app);
            std::fs::create_dir_all(&dir).unwrap();
            let mut info = offer();
            info["instanceId"] = json!(instance);
            info["updatedAt"] = json!(updated);
            std::fs::write(dir.join("bridge.json"), info.to_string()).unwrap();
        };
        write("studio", "old", "2026-11-25T01:00:00.000Z");
        write("studio-dev", "new", "2026-11-25T02:00:00.000Z");
        std::fs::create_dir_all(root.join("@paulga").join("empty")).unwrap();
        assert_eq!(find(&root, &[]).unwrap().instance, "new");
        assert_eq!(find(&root, &["new".into()]).unwrap().instance, "old");
        assert_eq!(find(&root, &["new".into(), "old".into()]), None);
        assert_eq!(find(&root.join("missing"), &[]), None);
        assert!(!studio_without_listener(&root), "both Studios listen for PAULGA Photo");
        // An older Studio is running too: it is up, but has no listener for this program.
        let older = root.join("@paulga").join("older");
        std::fs::create_dir_all(&older).unwrap();
        let running = json!({ "sessionToken": "t", "desktopPid": 7, "status": { "blender": { "listening": true, "endpoint": "ws://127.0.0.1:38350" } } });
        std::fs::write(older.join("bridge.json"), running.to_string()).unwrap();
        assert!(studio_without_listener(&root));
        let mut gone = running.clone();
        gone["shutdownIntent"] = json!("user_exit");
        std::fs::write(older.join("bridge.json"), gone.to_string()).unwrap();
        assert!(!studio_without_listener(&root), "a Studio that exited is not one that needs an update");
        let _ = std::fs::remove_dir_all(&root);
    }
}
