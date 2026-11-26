# PAULGA Studio connection

PAULGA Photo connects to the PAULGA Studio desktop app on the same computer, so Studio can read
the open document, capture a picture of it and run PAULGA Photo commands in it. The connection is
built into the Windows desktop app: there is nothing to install.

## Using it

- **Window ▸ PAULGA Studio** opens and closes the panel. It floats, docks with the other panels
  and remembers its place like them (panel id `studio`).
- The panel shows the state (Off, Looking for PAULGA Studio, Connecting, Connected, refused), the
  document Studio sees, Studio's version and port, and how many tools Studio has run.
- **Connect / Disconnect** switches the connection. The choice is kept across restarts (the
  preferences' `sections.paulgaStudio.connect`); it is on by default. **Try Again** looks for
  Studio at once.
- The status line says when the connection comes up, drops or is refused, and names each tool
  that changed the document.

The same through the automation interface:

| Command | Result |
|---|---|
| `window.panel.studio` `{show?}` | Show or hide the panel (no `show`: toggle). |
| `studio.connect` / `studio.disconnect` | Switch the connection; replies as `studio.status`. |
| `studio.status` | `{available, wanted, state, message, port, studioVersion, toolsRun, lastTool, document, panelOpen}`. |
| `studio.tool` `{name, parameters?, expectedRuntimeDocumentId?}` | Run a tool exactly as Studio's connection does. |

These are shell commands (`photocraft_engine::shell_commands::SHELL_COMMANDS`): over the control
channel `engine.execute {command, params}` runs the first three rows, and `studio.tool` is a
control method of its own (`docs/control-protocol.md`), which answers a tool that starts a
background job with that job's result when it ends.

On the web build and on desktops Studio does not run on, `studio.status` replies
`available: false` and the others fail with that reason; the panel says the same.

## How it connects

1. Studio writes `%APPDATA%\@paulga\<app>\bridge.json`: a session token, its process id and, per
   program, whether it listens and on which loopback port (`status["paulga-photo"]`, ports
   38370–38379).
2. PAULGA Photo reads the freshest file whose Studio has not marked itself as exited and connects
   only to the port recorded for it. Other ports are never probed.
3. Before the token is sent, it checks that the process on that port is the Studio the file names
   (`desktopPid`). A stale file cannot hand the token to another program.
4. It signs in (`x-paulga-session` header, then `auth`), and sends `cad:status` with the open
   document whenever that changes. Studio names every program's messages `cad:…`.

A dropped connection is retried after 1, 2, 4, 8, 16 s. A Studio that said it is exiting is not
tried again; a refused token is not sent again until Studio restarts or Connect is pressed.

Code: `apps/photocraft/src/paulga_studio/` (the socket), `crates/ui-egui/src/studio.rs` (state,
commands), `studio/tools.rs` (tools), `studio/panel.rs` (panel). The Nightly profile, which runs
without outside control, does not start the connection.

## Tools

`get-capabilities` lists exactly these eight; Studio's allowlist for `paulga-photo` names the
same. Sizes and positions are document pixels. There is no script tool: nothing runs code Studio
sent.

| Tool | Access | What it does |
|---|---|---|
| `get-capabilities` | read | This list, with parameter schemas. |
| `get-context` | read | Open documents and the active one: name, path, size, colour mode, bit depth, resolution, layer count, whether a pixel selection exists. |
| `get-selection` | read | The selected layers (`selectedIds` as bucket ids, `selectedLayers` with id, name, kind), the active layer, `hasPixelSelection` and `selectionBounds`. |
| `query-buckets` | read | Every layer, top to bottom, as a bucket `{id: "layer:<id>", name, kind: "layer", visible, meta}`; `meta` has the layer kind, opacity, blend mode, the group it is in (`parent`, `depth`) and its bounds. |
| `list-commands` | read | The command catalog for `run-command`: id, label, parameters, whether enabled; `filter`, `limit` (60, at most 1000). |
| `capture-view` | read | PNG as base64 in `image`, with `width`, `height`, `mimeType`. `mode: current` is the part of the document the window shows, `whole` the whole document; `longEdgePx` 256–8192 (2560). Never larger than the document. |
| `run-command` | change | One engine command with JSON `params`. Never opens a dialog. |
| `undo` | change | Undo `count` changes (default 1). |

Rules every tool follows:

- **One document.** A request that carries `expectedRuntimeDocumentId` runs only if that document
  is still the open one (`DOCUMENT_FENCE_MISMATCH` otherwise).
- **The person first.** A change is refused while a dialog or full-window workspace is open, a
  transform or crop is pending, type is being edited or a stroke is in progress (`CAD_BUSY`);
  reads still answer.
- **No code.** `run-command` refuses playing or loading actions, Batch and droplets, scripts and
  the Script Events Manager, every `plugin.*` command but `plugin.list`, the preferences that name
  scripts or plug-ins, and quitting (`COMMAND_REFUSED`), in any letter case. It runs only commands
  `list-commands` lists; the window's own commands, which may open a dialog or a file picker, are
  `COMMAND_UNKNOWN`.
- **Undo.** Each change is an ordinary undo step.
- Results over 50 MB are answered as a failure (`RESULT_TOO_LARGE`), not dropped.

## Not yet

- `run-command` can open, save and export files by path. Files are not limited to Studio's
  folders, and the automation folders of Preferences › AI Integrations do not apply to Studio.
- `capture-view` composites on the CPU in the document's own colours: no proof colours, no view
  rotation, and no overlays (selection edges, guides, a transform box).
- A tool runs on the interface thread; a long command that is not a background job holds the
  window until it ends.
- The Studio check is by process id; Studio's code signature is not verified.
- macOS and Linux: Studio writes its connection file only on Windows.
