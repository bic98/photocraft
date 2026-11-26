//! The PAULGA Studio link: what the desktop host's connection and the interface share, the
//! commands that open, close and switch it, and the tools Studio runs ([`tools`]).
//!
//! The host (`apps/photocraft`) owns the socket. It reads [`Link::snapshot`] to learn whether a
//! connection is wanted and which document is open, writes back how the connection is doing, and
//! runs each tool Studio asks for on the UI thread through the control method `studio.tool`.

pub mod exchange;
pub mod panel;
pub mod tools;

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The name Studio knows this program by (`status["paulga-photo"]` in its connection file).
pub const PLATFORM: &str = "paulga-photo";

/// The program's name as Studio and the person read it.
pub const DISPLAY_NAME: &str = "PAULGA Photo";

/// The panel's id in the dock, and the suffix of its Window menu command.
pub const PANEL: &str = "studio";

/// Window ▸ PAULGA Studio: shows or hides the panel.
pub const PANEL_COMMAND: &str = "window.panel.studio";

/// The preferences section that keeps the switch across restarts.
pub const PREFS_SECTION: &str = "paulgaStudio";

/// How the connection is doing, as the host last reported it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The person switched the connection off.
    #[default]
    Off,
    /// No running Studio was found; the host keeps looking.
    Searching,
    /// A Studio was found and the sign-in is under way.
    Connecting,
    Connected,
    /// Studio refused the sign-in; the host waits for a new session or for Connect.
    Refused,
}

impl Phase {
    pub fn id(self) -> &'static str {
        match self {
            Phase::Off => "off",
            Phase::Searching => "searching",
            Phase::Connecting => "connecting",
            Phase::Connected => "connected",
            Phase::Refused => "refused",
        }
    }
    /// The state as the panel and the status command word it (English; translated when shown).
    pub fn label(self) -> &'static str {
        match self {
            Phase::Off => "Off",
            Phase::Searching => "Looking for PAULGA Studio",
            Phase::Connecting => "Connecting",
            Phase::Connected => "Connected",
            Phase::Refused => "PAULGA Studio refused the connection",
        }
    }
}

/// The open document as Studio is told about it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Document {
    pub name: Option<String>,
    pub path: Option<String>,
    /// `<process>:<serial>`: names this run of the program and the document it shows. A request
    /// that carries another id was meant for a document that is no longer the open one.
    pub runtime_id: String,
    /// The person is in the middle of something (a dialog, a transform, a stroke), so a tool that
    /// changes the document would be refused.
    pub busy: bool,
}

#[derive(Clone, Debug, Default, Hash)]
pub struct State {
    /// The person wants a connection (the panel's switch, `studio.connect`).
    pub wanted: bool,
    /// Bumped by Connect, so a host waiting after a refusal tries again at once.
    pub attempt: u64,
    pub phase: Phase,
    /// Why the connection is not up, in English.
    pub note: Option<String>,
    pub port: Option<u16>,
    pub studio_version: Option<String>,
    pub tools_run: u64,
    pub last_tool: Option<String>,
    pub document: Document,
}

/// What tells one open document from another: its id in the session, its name and its file.
type DocumentKey = (u64, String, Option<String>);

/// Shared by the host's connection thread and the interface.
pub struct Link {
    state: Mutex<State>,
    /// `<pid>-<start, ms since 1970>`: one run of this program.
    process: String,
    /// What the open document was last time, and the serial it got.
    identity: Mutex<(Option<DocumentKey>, u32)>,
    /// The state the person was last told about ([`frame`]).
    told: Mutex<Phase>,
}

impl Link {
    pub fn new(wanted: bool) -> Arc<Self> {
        let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let link = Link {
            state: Mutex::new(State { wanted, ..State::default() }),
            process: format!("{}-{started}", std::process::id()),
            identity: Mutex::new((None, 1)),
            told: Mutex::new(Phase::Off),
        };
        link.edit(|s| s.document.runtime_id = format!("{}:1", link.process));
        Arc::new(link)
    }

    pub fn snapshot(&self) -> State {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn edit(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// The host says how the connection is doing.
    pub fn report(&self, phase: Phase, note: Option<String>) {
        self.edit(|s| {
            s.phase = phase;
            s.note = note;
            if phase != Phase::Connected {
                s.port = None;
            }
        });
    }

    /// The host is signed in to the Studio listening on `port`.
    pub fn connected(&self, port: u16, studio_version: Option<String>) {
        self.edit(|s| {
            s.phase = Phase::Connected;
            s.note = None;
            s.port = Some(port);
            s.studio_version = studio_version;
        });
    }

    /// The host ran a tool for Studio.
    pub fn ran(&self, tool: &str) {
        self.edit(|s| {
            s.tools_run = s.tools_run.saturating_add(1);
            s.last_tool = Some(tool.to_string());
        });
    }

    pub fn set_wanted(&self, wanted: bool) {
        self.edit(|s| {
            s.wanted = wanted;
            s.attempt = s.attempt.wrapping_add(1);
            // What the panel shows at once; the host reports what it finds within a moment.
            if !wanted {
                s.phase = Phase::Off;
                s.note = None;
                s.port = None;
            } else if s.phase == Phase::Off || s.phase == Phase::Refused {
                s.phase = Phase::Searching;
                s.note = None;
            }
        });
    }

    /// Record the document the app shows now. A different document gets the next serial.
    fn publish(&self, key: Option<DocumentKey>, busy: bool) {
        let serial = {
            let mut identity = self.identity.lock().unwrap_or_else(PoisonError::into_inner);
            if identity.0 != key {
                identity.0 = key.clone();
                identity.1 = identity.1.wrapping_add(1).max(1);
            }
            identity.1
        };
        let document = Document {
            name: key.as_ref().map(|(_, name, _)| name.clone()),
            path: key.and_then(|(_, _, path)| path),
            runtime_id: format!("{}:{serial}", self.process),
            busy,
        };
        self.edit(|s| {
            if s.document != document {
                s.document = document;
            }
        });
    }
}

/// What the person is in the middle of, when a change from outside would cut across it: an open
/// dialog or full-window workspace, a transform, a crop, type being edited, or a stroke or drag on
/// the canvas. English; `None` when the document is free.
pub fn busy(app: &PhotocraftApp) -> Option<&'static str> {
    if app.dialog_open() || crate::prefs_ui::shortcuts_open(app) {
        Some("a dialog is open")
    } else if app.smart_svg.modal_open() || crate::distort_ui::modal_workspace(app).is_some() {
        Some("a full-window workspace is open")
    } else if app.ui.transform.is_some() {
        Some("a transform is open")
    } else if crate::crop_ui::pending(app) {
        Some("a crop is waiting to be applied")
    } else if app.ui.text_edit.is_some() {
        Some("type is being edited")
    } else if crate::canvas::tools::in_gesture(app) {
        Some("a stroke or drag is in progress")
    } else {
        None
    }
}

/// Tell the link which document is open. Cheap; runs every frame and before each tool.
pub fn publish(app: &PhotocraftApp) {
    let Some(link) = &app.studio else { return };
    let key = app.session.active().map(|d| (d.doc.id.0, d.doc.name.clone(), d.path.clone()));
    link.publish(key, busy(app).is_some());
}

/// Once a frame: publish the open document, and say in the status line when the connection comes
/// up, drops or is refused. It changes without the person doing anything, wherever they look.
pub fn frame(app: &mut PhotocraftApp) {
    publish(app);
    let Some(link) = app.studio.clone() else { return };
    let now = link.snapshot().phase;
    let before = std::mem::replace(&mut *link.told.lock().unwrap_or_else(PoisonError::into_inner), now);
    let message = match (before, now) {
        (a, b) if a == b => return,
        (_, Phase::Connected) => tl!("Connected to PAULGA Studio"),
        (_, Phase::Refused) => tl!("PAULGA Studio refused the connection"),
        (Phase::Connected, _) => tl!("Disconnected from PAULGA Studio"),
        _ => return,
    };
    say(app, message.to_string());
}

/// Put `message` in the status line.
pub(crate) fn say(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message;
    app.ui.status_error = false;
}

/// What the panel shows of the link, for the panel's memo key: the host changes it with no command.
pub(crate) fn panel_key(app: &PhotocraftApp) -> Option<State> {
    app.studio.as_ref().map(|link| link.snapshot())
}

/// The ids this module answers: the `studio.*` commands and the panel's Window menu command.
pub fn handles(id: &str) -> bool {
    matches!(id, "studio.connect" | "studio.disconnect" | "studio.status" | "studio.tool") || id == PANEL_COMMAND
}

/// `studio.status`: the connection as the panel shows it.
pub fn status(app: &PhotocraftApp) -> Value {
    let panel_open = crate::panel_docking::is_shown(app, PANEL);
    let Some(link) = &app.studio else {
        return json!({ "available": false, "state": Phase::Off.id(), "message": UNAVAILABLE, "panelOpen": panel_open });
    };
    publish(app);
    let s = link.snapshot();
    json!({
        "available": true,
        "wanted": s.wanted,
        "state": s.phase.id(),
        "message": s.note.clone().unwrap_or_else(|| s.phase.label().to_string()),
        "port": s.port,
        "studioVersion": s.studio_version,
        "toolsRun": s.tools_run,
        "lastTool": s.last_tool,
        "document": { "name": s.document.name, "path": s.document.path, "runtimeDocumentId": s.document.runtime_id },
        "panelOpen": panel_open,
    })
}

/// What a build without the connection says (the web build, and desktops Studio does not run on).
pub const UNAVAILABLE: &str = "The PAULGA Studio connection is part of the Windows desktop app.";

/// Whether the saved preferences ask for a connection: on unless the person switched it off.
pub fn wanted_in(prefs: &photocraft_engine::prefs::Preferences) -> bool {
    prefs.sections.get(PREFS_SECTION).and_then(|section| section.get("connect")).and_then(Value::as_bool).unwrap_or(true)
}

/// The `studio.*` commands; `None` for any other id. The panel's own command
/// ([`PANEL_COMMAND`]) is the dock's, like every panel's.
pub fn command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "studio.status" => Ok(status(app)),
        "studio.connect" | "studio.disconnect" => {
            let Some(link) = app.studio.clone() else { return Some(Err(UNAVAILABLE.into())) };
            let wanted = id == "studio.connect";
            link.set_wanted(wanted);
            // Kept across restarts; the preferences are written when they change.
            if wanted_in(app.session.prefs()) != wanted {
                app.session.edit_prefs(|prefs| prefs.sections.insert(PREFS_SECTION.into(), json!({ "connect": wanted })));
            }
            Ok(status(app))
        }
        // The automation interface runs the same tools Studio does, with the same checks.
        "studio.tool" => tools::call(app, params),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(Session::new(), crate::Services::default())
    }

    fn linked(link: &Arc<Link>) -> Box<PhotocraftApp> {
        let mut app = app();
        app.studio = Some(link.clone());
        app
    }

    fn new_document(app: &mut PhotocraftApp) {
        app.run("file.new", json!({ "width": 32, "height": 24 })).unwrap();
    }

    #[test]
    fn a_build_without_the_connection_says_so_instead_of_pretending() {
        let mut app = app();
        assert_eq!(app.run("studio.status", json!({})).unwrap()["available"], false);
        assert!(app.run("studio.connect", json!({})).is_err());
    }

    #[test]
    fn connect_and_disconnect_reach_the_host_wake_it_and_are_kept_for_the_next_start() {
        let link = Link::new(true);
        let mut app = linked(&link);
        assert!(wanted_in(app.session.prefs()), "on until the person switches it off");
        let before = link.snapshot().attempt;
        let off = app.run("studio.disconnect", json!({})).unwrap();
        assert_eq!(off["wanted"], false);
        assert!(!link.snapshot().wanted);
        assert!(!wanted_in(app.session.prefs()));
        app.run("studio.connect", json!({})).unwrap();
        let s = link.snapshot();
        assert!(s.wanted);
        assert_ne!(s.attempt, before);
        assert!(wanted_in(app.session.prefs()));
    }

    #[test]
    fn another_document_gets_another_runtime_id_and_the_same_one_keeps_it() {
        let link = Link::new(true);
        let mut app = linked(&link);
        new_document(&mut app);
        publish(&app);
        let first = link.snapshot().document.runtime_id;
        publish(&app);
        assert_eq!(link.snapshot().document.runtime_id, first);
        new_document(&mut app);
        publish(&app);
        let second = link.snapshot().document;
        assert_ne!(second.runtime_id, first);
        assert!(second.name.is_some());
    }

    #[test]
    fn the_person_is_told_when_the_connection_comes_up_and_when_it_drops() {
        let link = Link::new(true);
        let mut app = linked(&link);
        frame(&mut app);
        assert!(app.ui.status.is_empty(), "nothing to say while it only looks for Studio");
        link.connected(38370, None);
        frame(&mut app);
        let up = std::mem::take(&mut app.ui.status);
        assert!(!up.is_empty());
        link.report(Phase::Searching, None);
        frame(&mut app);
        let down = std::mem::take(&mut app.ui.status);
        assert!(!down.is_empty());
        assert_ne!(up, down);
        frame(&mut app);
        assert!(app.ui.status.is_empty(), "said once");
    }

    #[test]
    fn the_panel_opens_and_closes_from_its_window_menu_command() {
        let mut app = app();
        let ctx = app.ctx().clone();
        let shown = |app: &PhotocraftApp| crate::panel_docking::is_shown(app, PANEL);
        assert!(!shown(&app), "closed until the person opens it");
        assert!(crate::menus::is_enabled(&app, PANEL_COMMAND), "its Window menu row is live with no document open");
        let rows = crate::menus::menu_rows(&app);
        assert_eq!(crate::menus::row(&rows, PANEL_COMMAND).map(|row| row.path.clone()), Some(vec!["Window".to_string()]));
        crate::menus::invoke(&mut app, &ctx, PANEL_COMMAND, json!({})).unwrap();
        assert!(shown(&app));
        assert_eq!(app.run("studio.status", json!({})).unwrap()["panelOpen"], true);
        crate::menus::invoke(&mut app, &ctx, PANEL_COMMAND, json!({})).unwrap();
        assert!(!shown(&app));
        for _ in 0..2 {
            crate::menus::invoke(&mut app, &ctx, PANEL_COMMAND, json!({ "show": true })).unwrap();
        }
        assert!(shown(&app), "opening twice keeps it open");
    }

    #[test]
    fn the_panels_view_follows_what_the_host_reports() {
        let link = Link::new(true);
        let app = linked(&link);
        let before = crate::panels::region_versions::panel(&app, PANEL);
        link.connected(38370, Some("1.2.3".into()));
        assert_ne!(crate::panels::region_versions::panel(&app, PANEL), before, "the panel is drawn again when the connection comes up");
    }
}
