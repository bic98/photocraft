//! PAULGA Photo's connection to PAULGA Studio on the same computer (Windows).
//!
//! One background thread finds the running Studio through its connection file ([`bridge`]), signs
//! in over a loopback WebSocket and then serves Studio's requests ([`wire`]). Each tool Studio asks
//! for runs on the UI thread as the control method `studio.tool`
//! (`photocraft_ui_egui::studio::tools`), so this thread never touches the document.
//!
//! The person switches the connection in the PAULGA Studio panel; this thread follows
//! [`Link::snapshot`] and reports back through [`Link::report`].

mod bridge;
mod wire;

use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use photocraft_ui_egui::ControlRequest;
use photocraft_ui_egui::studio::{Document, Link, Phase};
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

type Failure = Box<dyn std::error::Error + Send + Sync>;
type Socket = WebSocket<TcpStream>;

/// How often the thread looks at the link while it waits for Studio or for a message.
const POLL: Duration = Duration::from_millis(200);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Studio pings every 30 s and gives up after three unanswered; this long without a word from
/// it means the connection is dead.
const SILENCE_LIMIT: Duration = Duration::from_secs(150);
/// Seconds between tries after a failure, the last one repeating.
const RETRY_SECS: [u64; 5] = [1, 2, 4, 8, 16];
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOOL_TIMEOUT: Duration = Duration::from_secs(600);
/// The largest request read from Studio (tool parameters).
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
/// `CREATE_NO_WINDOW`: no console flashes up for the socket-owner check.
const NO_WINDOW: u32 = 0x0800_0000;

/// Why a connection ended without an error.
enum End {
    /// The person switched the connection off.
    Off,
    /// The socket closed; Studio may still be there.
    Closed,
    /// Studio said it is exiting.
    StudioExited,
    /// Studio refused the session token.
    Refused(String),
}

/// The tools Studio may have waiting for the UI thread at once; one more is answered as busy.
const QUEUE: usize = 4;

/// Start the connection thread. The receiver carries the tools Studio asks for, as control
/// requests for the app to run (`PhotocraftApp::with_studio`).
pub(crate) fn start(ctx: egui::Context, link: Arc<Link>) -> Receiver<ControlRequest> {
    let (tx, rx) = sync_channel(QUEUE);
    let thread_link = link.clone();
    let spawned = std::thread::Builder::new().name("paulga-studio".into()).spawn(move || supervise(&ctx, &thread_link, &tx));
    if let Err(error) = spawned {
        log::error!("PAULGA Studio connection not started: {error}");
        link.report(Phase::Off, Some(format!("The connection could not start: {error}")));
    }
    rx
}

/// Report a state once, and repaint so the panel and the status line show it.
fn tell(ctx: &egui::Context, link: &Link, phase: Phase, note: Option<String>) {
    let now = link.snapshot();
    if now.phase != phase || now.note != note {
        link.report(phase, note);
        ctx.request_repaint();
    }
}

/// Sleep up to `time`, returning early when the person presses Connect or Disconnect.
fn pause(link: &Link, attempt: u64, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until && link.snapshot().attempt == attempt {
        std::thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
    }
}

fn supervise(ctx: &egui::Context, link: &Link, tx: &SyncSender<ControlRequest>) {
    let Some(root) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        link.report(Phase::Off, Some("APPDATA is not set, so PAULGA Studio cannot be found.".into()));
        return;
    };
    // The session Studio refused, and the Studio runs that said they were exiting.
    let mut refused: Option<(String, String)> = None;
    let mut exited: Vec<String> = Vec::new();
    let mut failures = 0usize;
    let mut attempt = link.snapshot().attempt;
    loop {
        let state = link.snapshot();
        if state.attempt != attempt {
            // Connect was pressed: forget the refusal and try now.
            attempt = state.attempt;
            refused = None;
            failures = 0;
        }
        if !state.wanted {
            tell(ctx, link, Phase::Off, None);
            pause(link, attempt, Duration::from_secs(3600));
            continue;
        }
        let Some(found) = bridge::find(&root, &exited) else {
            let why = if bridge::studio_without_listener(&root) {
                "The PAULGA Studio that is running does not connect to PAULGA Photo. Update PAULGA Studio."
            } else {
                "PAULGA Studio is not running. The connection starts when it does."
            };
            tell(ctx, link, Phase::Searching, Some(why.into()));
            pause(link, attempt, Duration::from_secs(2));
            continue;
        };
        if refused.as_ref() == Some(&found.session()) {
            pause(link, attempt, Duration::from_secs(2));
            continue;
        }
        tell(ctx, link, Phase::Connecting, None);
        let wait = match serve(ctx, link, tx, &root, &found) {
            Ok(End::Off) => Duration::ZERO,
            Ok(End::Closed) => {
                failures = 0;
                tell(ctx, link, Phase::Searching, Some("The connection to PAULGA Studio closed. Trying again.".into()));
                Duration::from_secs(1)
            }
            Ok(End::StudioExited) => {
                exited.push(found.instance.clone());
                if exited.len() > 8 {
                    exited.remove(0);
                }
                tell(ctx, link, Phase::Searching, Some("PAULGA Studio closed. The connection starts again when it does.".into()));
                Duration::from_secs(1)
            }
            Ok(End::Refused(why)) => {
                refused = Some(found.session());
                tell(ctx, link, Phase::Refused, Some(why));
                Duration::ZERO
            }
            Err(error) => {
                log::warn!("PAULGA Studio connection: {error}");
                let secs = RETRY_SECS[failures.min(RETRY_SECS.len() - 1)];
                failures += 1;
                tell(ctx, link, Phase::Searching, Some(format!("{error}. Trying again in {secs} s.")));
                Duration::from_secs(secs)
            }
        };
        pause(link, attempt, wait);
    }
}

/// Check that the process on the other end of `stream` is the Studio the connection file names,
/// before the session token is sent: a stale file must not hand it to whatever took the port.
fn owned_by(stream: &TcpStream, desktop_pid: u32) -> Result<(), Failure> {
    let netstat = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?).join("System32").join("netstat.exe");
    let table = Command::new(netstat).args(["-ano", "-p", "tcp"]).creation_flags(NO_WINDOW).output()?;
    let (theirs, ours, pid) = (stream.peer_addr()?.to_string(), stream.local_addr()?.to_string(), desktop_pid.to_string());
    // Studio's row for this connection: its address, ours, the state, its process id.
    let found = table.status.success()
        && String::from_utf8_lossy(&table.stdout).lines().any(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            fields.len() == 5 && fields[0] == "TCP" && fields[1] == theirs && fields[2] == ours && fields[4] == pid
        });
    if found { Ok(()) } else { Err("The program on PAULGA Studio's port is not PAULGA Studio".into()) }
}

fn send(socket: &mut Socket, text: String) -> Result<(), Failure> {
    socket.send(Message::text(text))?;
    Ok(())
}

/// One connection, from the TCP connect until it ends.
fn serve(ctx: &egui::Context, link: &Link, tx: &SyncSender<ControlRequest>, root: &Path, bridge: &bridge::Bridge) -> Result<End, Failure> {
    let stream = TcpStream::connect_timeout(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, bridge.port).into(), CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(SIGN_IN_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let _ = stream.set_nodelay(true);
    owned_by(&stream, bridge.desktop_pid)?;
    let mut request = format!("ws://127.0.0.1:{}", bridge.port).into_client_request()?;
    request.headers_mut().insert("x-paulga-session", bridge.token.parse()?);
    let config = tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_REQUEST_BYTES)).max_frame_size(Some(MAX_REQUEST_BYTES));
    let (mut socket, _) = tungstenite::client::client_with_config(request, stream, Some(config)).map_err(|error| format!("PAULGA Studio did not accept the connection ({error})"))?;
    socket.get_ref().set_read_timeout(Some(POLL))?;

    let (mut signed_in, mut told): (bool, Option<Document>) = (false, None);
    let (opened, mut heard) = (Instant::now(), Instant::now());
    loop {
        let state = link.snapshot();
        if !state.wanted {
            let _ = socket.close(None);
            let _ = socket.flush();
            return Ok(End::Off);
        }
        // Studio restarted or exited under this connection: its file says so before the socket does.
        if signed_in && told.as_ref() != Some(&state.document) {
            send(&mut socket, wire::envelope("cad:status", wire::status(&state.document)))?;
            told = Some(state.document.clone());
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if !signed_in && opened.elapsed() > SIGN_IN_TIMEOUT {
                    return Err("PAULGA Studio did not finish the sign-in".into());
                }
                if heard.elapsed() > SILENCE_LIMIT {
                    return Err("PAULGA Studio stopped answering".into());
                }
                if signed_in && bridge::find(root, &[]).is_none_or(|current| current.instance != bridge.instance) && heard.elapsed() > Duration::from_secs(5) {
                    return Ok(End::Closed);
                }
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => return Ok(End::Closed),
            Err(error) => return Err(error.into()),
        };
        heard = Instant::now();
        if message.is_close() {
            return Ok(End::Closed);
        }
        let Ok(frame) = message.to_text().map_err(drop).and_then(|text| serde_json::from_str::<Value>(text).map_err(drop)) else { continue };
        let payload = &frame["payload"];
        match frame["type"].as_str().unwrap_or_default() {
            "auth:required" if !signed_in => send(&mut socket, wire::envelope("auth", json!({ "token": bridge.token })))?,
            "auth:success" if !signed_in => {
                signed_in = true;
                link.connected(bridge.port, bridge.desktop_version.clone());
                ctx.request_repaint();
            }
            "auth:error" => {
                let why = payload["message"].as_str().unwrap_or("PAULGA Studio refused the session.");
                return Ok(End::Refused(format!("{why} Press Connect to try again.")));
            }
            "ping" => send(&mut socket, wire::envelope("pong", json!({})))?,
            "disconnect" => return Ok(if wire::shutting_down(payload) { End::StudioExited } else { End::Closed }),
            "cad:execute-tool" if signed_in => {
                if let Some(answer) = execute(ctx, link, tx, &mut socket, payload)? {
                    send(&mut socket, answer)?;
                }
            }
            "cad:request-data" if signed_in => send(&mut socket, wire::data_unsupported(&payload["requestId"]))?,
            // `cad:cancel-tool` has nothing to cancel (a tool is answered before the next message
            // is read); acknowledgements and anything newer are ignored.
            _ => {}
        }
    }
}

/// How long Studio waits for this request, from the deadline it sent.
fn patience(payload: &Value) -> Duration {
    let Some(deadline) = payload["executionDeadlineUnixMs"].as_u64() else { return TOOL_TIMEOUT };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Duration::from_millis(deadline.saturating_sub(now)).clamp(Duration::from_secs(1), MAX_TOOL_TIMEOUT)
}

/// Run the tool `payload` asks for on the UI thread and build its answer. `None` when the
/// request has no usable id, so there is nothing to address an answer to.
///
/// Only the tool's name, its parameters and the document it was written for go to the app: this
/// program has no script tool, so an authorization Studio attached is not passed on.
fn execute(ctx: &egui::Context, link: &Link, tx: &SyncSender<ControlRequest>, socket: &mut Socket, payload: &Value) -> Result<Option<String>, Failure> {
    let Some(request_id) = wire::identifier(&payload["requestId"]) else { return Ok(None) };
    let started = Instant::now();
    let Some(tool) = wire::identifier(&payload["toolName"]) else {
        return Ok(Some(wire::tool_result(request_id, "unknown", Err("INVALID_TOOL_NAME: the tool name is not valid".into()), 0)));
    };
    send(socket, wire::envelope("cad:tool-started", json!({ "requestId": request_id, "toolName": tool })))?;
    let mut params = json!({ "name": tool, "parameters": payload["parameters"] });
    if let Some(expected) = payload["expectedRuntimeDocumentId"].as_str() {
        params["expectedRuntimeDocumentId"] = json!(expected);
    }
    let wait = patience(payload);
    let (request, reply) = ControlRequest::new("studio.tool", params);
    // Shared with the UI thread: when this side stops waiting, it says whether the tool had started.
    let dispatch = request.dispatch.clone();
    let outcome = if tx.try_send(request).is_err() {
        Err("CAD_BUSY: PAULGA Photo has other requests waiting; nothing was run, so sending it again is safe".to_string())
    } else {
        ctx.request_repaint();
        match reply.recv_timeout(wait) {
            Ok(answer) if answer["ok"] == true => Ok(answer["result"].clone()),
            Ok(answer) => Err(answer["error"].as_str().unwrap_or("failed").to_string()),
            Err(RecvTimeoutError::Timeout) if dispatch.give_up() == craft::automation::control::expired_error() => {
                Err(format!("TIMEOUT_NOT_RUN: PAULGA Photo did not start {tool} within {} s; nothing was run", wait.as_secs()))
            }
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "TIMEOUT_MAY_HAVE_RUN: PAULGA Photo did not finish {tool} within {} s; it may still change the document, so read it before sending again",
                wait.as_secs()
            )),
            Err(RecvTimeoutError::Disconnected) => Err(format!("NOT_ANSWERED: PAULGA Photo is closing and did not answer {tool}; read the document before sending again")),
        }
    };
    link.ran(tool);
    ctx.request_repaint();
    Ok(Some(wire::tool_result(request_id, tool, outcome, started.elapsed().as_millis())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    type Studio = WebSocket<TcpStream>;

    /// A stand-in Studio: its connection file under a folder of its own, and its listener. The
    /// file names this test process, which is the one that owns the port.
    fn studio(name: &str) -> (PathBuf, TcpListener, bridge::Bridge) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = std::env::temp_dir().join(format!("paulga-photo-session-{}-{name}", std::process::id()));
        let dir = root.join("@paulga").join("studio");
        std::fs::create_dir_all(&dir).unwrap();
        let info = json!({
            "sessionToken": "tok", "instanceId": name, "desktopPid": std::process::id(), "desktopVersion": "9.9.9",
            "updatedAt": "2026-11-25T00:00:00.000Z",
            "status": { "paulga-photo": { "listening": true, "endpoint": format!("ws://127.0.0.1:{port}") } }
        });
        std::fs::write(dir.join("bridge.json"), info.to_string()).unwrap();
        let found = bridge::find(&root, &[]).unwrap();
        (root, listener, found)
    }

    /// Notes the session header a connection presents.
    struct SessionHeader<'a>(&'a mut Option<String>);

    impl tungstenite::handshake::server::Callback for SessionHeader<'_> {
        fn on_request(
            self,
            request: &tungstenite::handshake::server::Request,
            response: tungstenite::handshake::server::Response,
        ) -> Result<tungstenite::handshake::server::Response, tungstenite::handshake::server::ErrorResponse> {
            *self.0 = request.headers().get("x-paulga-session").and_then(|v| v.to_str().ok()).map(str::to_string);
            Ok(response)
        }
    }

    /// Accept PAULGA Photo's connection, checking the session header it presents.
    fn accept(listener: &TcpListener) -> Studio {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut presented = None;
        let studio = tungstenite::accept_hdr(stream, SessionHeader(&mut presented)).unwrap();
        assert_eq!(presented.as_deref(), Some("tok"));
        studio
    }

    fn say(studio: &mut Studio, kind: &str, payload: Value) {
        studio.send(Message::text(json!({ "type": kind, "payload": payload }).to_string())).unwrap();
    }

    /// The next message PAULGA Photo sends: `(type, payload)`.
    fn hear(studio: &mut Studio) -> (String, Value) {
        let frame: Value = serde_json::from_str(studio.read().unwrap().to_text().unwrap()).unwrap();
        assert!(frame["timestamp"].is_string());
        (frame["type"].as_str().unwrap().to_string(), frame["payload"].clone())
    }

    #[test]
    fn a_session_signs_in_reports_the_document_runs_tools_and_ends_when_switched_off() {
        let (root, listener, found) = studio("serve");
        let link = Link::new(true);
        let (tx, rx) = sync_channel::<ControlRequest>(4);
        // The app: answers each tool with what it was asked.
        let app = std::thread::spawn(move || {
            for request in rx {
                assert_eq!(request.method, "studio.tool");
                assert!(request.dispatch.start());
                let _ = request.reply.send(json!({ "ok": true, "result": { "asked": request.params } }));
            }
        });
        let photo = {
            let (link, root) = (link.clone(), root.clone());
            std::thread::spawn(move || serve(&egui::Context::default(), &link, &tx, &root, &found).map(|end| matches!(end, End::Off)).map_err(|e| e.to_string()))
        };
        let mut studio = accept(&listener);
        say(&mut studio, "auth:required", json!({}));
        assert_eq!(hear(&mut studio), ("auth".to_string(), json!({ "token": "tok" })));
        say(&mut studio, "auth:success", json!({ "sessionId": "client-7" }));
        let (kind, status) = hear(&mut studio);
        assert_eq!(kind, "cad:status");
        assert_eq!(status["contextComplete"], true);
        assert_eq!(status["runtimeDocumentId"], link.snapshot().document.runtime_id);
        let connected = link.snapshot();
        assert_eq!((connected.phase, connected.studio_version.as_deref()), (Phase::Connected, Some("9.9.9")));

        say(&mut studio, "ping", json!({}));
        assert_eq!(hear(&mut studio).0, "pong");

        say(&mut studio, "cad:execute-tool", json!({ "toolName": "get-context", "requestId": "r-1", "parameters": { "a": 1 }, "expectedRuntimeDocumentId": "doc:7" }));
        assert_eq!(hear(&mut studio), ("cad:tool-started".to_string(), json!({ "requestId": "r-1", "toolName": "get-context" })));
        let (kind, result) = hear(&mut studio);
        assert_eq!(kind, "cad:tool-result");
        assert_eq!((result["requestId"].clone(), result["success"].clone()), (json!("r-1"), json!(true)));
        assert_eq!(result["result"]["asked"], json!({ "name": "get-context", "parameters": { "a": 1 }, "expectedRuntimeDocumentId": "doc:7" }));
        assert_eq!(link.snapshot().last_tool.as_deref(), Some("get-context"));

        // This program runs no scripts: an authorization Studio attached is not passed to the app.
        let grant = json!({ "enabled": true, "requestId": "r-s", "targetClientId": "client-7", "runtimeDocumentId": "d", "expiresAtUnixMs": 1 });
        say(&mut studio, "cad:execute-tool", json!({ "toolName": "execute-script", "requestId": "r-s", "parameters": { "code": "1" }, "scriptAuthorization": grant }));
        assert_eq!(hear(&mut studio).0, "cad:tool-started");
        let asked = hear(&mut studio).1["result"]["asked"].clone();
        assert_eq!(asked, json!({ "name": "execute-script", "parameters": { "code": "1" } }));

        // A tool name that is not one answers as a failure; a data request is declined, not ignored.
        say(&mut studio, "cad:execute-tool", json!({ "toolName": "../x", "requestId": "r-2" }));
        let (_, bad) = hear(&mut studio);
        assert_eq!((bad["requestId"].clone(), bad["success"].clone()), (json!("r-2"), json!(false)));
        say(&mut studio, "cad:request-data", json!({ "requestId": "d-1", "scope": "layers" }));
        let (kind, data) = hear(&mut studio);
        assert_eq!((kind.as_str(), data["requestId"].clone(), data["data"]["success"].clone()), ("cad:data", json!("d-1"), json!(false)));

        // Disconnect in the panel: the socket closes and the session ends as switched off.
        link.set_wanted(false);
        while studio.read().is_ok_and(|message| !message.is_close()) {}
        assert_eq!(photo.join().unwrap(), Ok(true));
        app.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_refused_token_and_an_exiting_studio_end_the_session_as_what_they_are() {
        let cases = [
            ("refused", "auth:error", json!({ "code": "INVALID_TOKEN", "message": "Invalid session token." })),
            ("exiting", "disconnect", json!({ "reason": "Desktop app shutting down" })),
        ];
        for (name, kind, payload) in cases {
            let (root, listener, found) = studio(name);
            let link = Link::new(true);
            let (tx, _rx) = sync_channel::<ControlRequest>(1);
            let photo = {
                let (link, root) = (link.clone(), root.clone());
                std::thread::spawn(move || serve(&egui::Context::default(), &link, &tx, &root, &found).map_err(|e| e.to_string()))
            };
            let mut studio = accept(&listener);
            say(&mut studio, kind, payload);
            match (name, photo.join().unwrap()) {
                ("refused", Ok(End::Refused(why))) => assert!(why.contains("Invalid session token")),
                ("exiting", Ok(End::StudioExited)) => {}
                (_, other) => panic!("{name}: {:?}", other.map(|_| "another end")),
            }
            assert_ne!(link.snapshot().phase, Phase::Connected);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn the_token_is_not_sent_to_a_port_another_process_is_said_to_own() {
        let (root, listener, mut found) = studio("impostor");
        found.desktop_pid = found.desktop_pid.wrapping_add(1);
        let link = Link::new(true);
        let (tx, _rx) = sync_channel::<ControlRequest>(1);
        let error = serve(&egui::Context::default(), &link, &tx, &root, &found).err().map(|e| e.to_string());
        assert_eq!(error.as_deref(), Some("The program on PAULGA Studio's port is not PAULGA Studio"));
        // Nothing but the TCP connect reached the listener: no request, no header, no token.
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let mut received = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stream, &mut received);
        assert!(received.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_request_waits_as_long_as_studio_does_within_bounds() {
        let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
        assert_eq!(patience(&json!({})), TOOL_TIMEOUT);
        let soon = patience(&json!({ "executionDeadlineUnixMs": now + 30_000 }));
        assert!(soon > Duration::from_secs(25) && soon <= Duration::from_secs(30));
        assert_eq!(patience(&json!({ "executionDeadlineUnixMs": now.saturating_sub(5_000) })), Duration::from_secs(1), "a past deadline still gets a moment");
        assert_eq!(patience(&json!({ "executionDeadlineUnixMs": now + 86_400_000 })), MAX_TOOL_TIMEOUT);
    }
}
