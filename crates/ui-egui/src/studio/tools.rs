//! The tools PAULGA Studio runs in this program. Each one is a thin face on registered commands,
//! so Studio can do nothing the command interface cannot, and every change is an ordinary undo
//! step.
//!
//! [`call`] is the single entry: the control method `studio.tool` and the command of the same
//! name both go through it, with the same checks.

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// What a tool does to the document; Studio asks its operator before a `Change`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Change,
}

pub struct Tool {
    pub name: &'static str,
    pub access: Access,
    pub domain: &'static str,
    pub risk: &'static str,
    pub description: &'static str,
    /// The parameters as JSON Schema properties: `(name, type, description)`.
    pub params: &'static [(&'static str, &'static str, &'static str)],
}

const fn read(name: &'static str, domain: &'static str, description: &'static str, params: &'static [(&'static str, &'static str, &'static str)]) -> Tool {
    Tool { name, access: Access::Read, domain, risk: "low", description, params }
}

const fn change(
    name: &'static str,
    domain: &'static str,
    risk: &'static str,
    description: &'static str,
    params: &'static [(&'static str, &'static str, &'static str)],
) -> Tool {
    Tool { name, access: Access::Change, domain, risk, description, params }
}

/// Every tool, in the order `get-capabilities` lists them. Studio's allowlist for this program
/// names exactly these. There is no script tool: nothing here runs code Studio sent.
pub const TOOLS: &[Tool] = &[
    read("get-capabilities", "system", "List the tools PAULGA Photo runs for PAULGA Studio.", &[]),
    read("get-context", "document", "The open documents and the active one: name, path, size in pixels, colour mode, bit depth, resolution, layer count.", &[]),
    read("get-selection", "object", "The selected layers (ids and names), the active layer, and whether a pixel selection exists.", &[]),
    read(
        "query-buckets",
        "layer",
        "The document's layers as buckets, top to bottom: id `layer:<id>`, name, whether shown, and kind, opacity, blend mode and group in `meta`.",
        &[],
    ),
    read(
        "list-commands",
        "system",
        "PAULGA Photo's command catalog for `run-command`: id, label and the JSON parameters each takes.",
        &[("filter", "string", "Only commands whose id, label or keywords contain this text."), ("limit", "integer", "Most commands returned (default 60).")],
    ),
    read(
        "capture-view",
        "view",
        "A PNG of the document: the part the window shows, or the whole document.",
        &[("mode", "string", "`current` (default) or `whole`."), ("longEdgePx", "integer", "Long edge in pixels, 256 to 8192 (default 2560). The picture is never larger than the document.")],
    ),
    change(
        "run-command",
        "system",
        "high",
        "Run one PAULGA Photo command with JSON parameters (see `list-commands`). Never opens a dialog. Commands that run scripts, actions or plug-ins, or quit the program, are refused.",
        &[("command", "string", "Command id, e.g. `layer.new.layer`, `filter.blur.gaussianBlur`."), ("params", "object", "The command's parameters.")],
    ),
    change("undo", "system", "medium", "Undo the last change, or the last `count` changes.", &[("count", "integer", "Steps to undo (default 1).")]),
    change(
        "apply-layer-visibility",
        "layer",
        "low",
        "Show or hide layers: `states` [{name, visible}] (a layer's name, or its bucket id `layer:<id>` as `id`), or `showAll: true`.",
        &[("states", "array", "[{name, visible}] for each layer to change."), ("showAll", "boolean", "Show every layer, before `states`.")],
    ),
    read(
        "export-selected-mesh",
        "object",
        "Export the active document as a textured 2D quad in a PAULGA Send Mode mesh bundle.",
        &[("bundleFilePath", "string", "Optional file path to write the bundle JSON to.")],
    ),
    change(
        "import-mesh-bundle",
        "object",
        "medium",
        "Import a PAULGA Send Mode mesh bundle (curves, meshes, images) as embedded layers.",
        &[
            ("bundle", "object", "Inline Send Mode bundle JSON."),
            ("bundleFilePath", "string", "Path to a Send Mode bundle JSON file."),
            ("bundleFilePaths", "array", "Chunked Send Mode bundle JSON file paths."),
        ],
    ),
];

/// Commands `run-command` does not run: they play actions (many commands the request does not
/// spell out, also from files), run scripts on events, or end the session.
const REFUSED: &[&str] = &[
    "actions.play",
    "actions.load",
    "actions.replace",
    "file.automate.batch",
    "file.automate.createDroplet",
    "file.automate.runDroplet",
    "file.scripts.browse",
    "file.scripts.scriptEventsManager",
    "file.exit",
    "app.quit",
    "studio.tool",
    "studio.connect",
    "studio.disconnect",
];

/// Plug-in commands (`plugin.install`, `plugin.run`, `plugin.filter.<id>`, …) run or bring in
/// code; listing the installed ones does not.
const PLUGIN_PREFIX: &str = "plugin.";
const PLUGIN_LIST: &str = "plugin.list";

/// Preference sections that name scripts to run or plug-ins to load.
const REFUSED_PREFS: &[&str] = &["scriptevents", "plugins"];

const DEFAULT_LONG_EDGE: f64 = 2560.0;
const LONG_EDGE: std::ops::RangeInclusive<f64> = 256.0..=8192.0;
const MAX_UNDO: u64 = 1000;

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// What `get-capabilities` answers: one descriptor per tool.
pub fn capabilities() -> Value {
    let operations: Vec<Value> = TOOLS
        .iter()
        .map(|tool| {
            let verb = tool.name.replace('-', "_");
            let properties: Map<String, Value> =
                tool.params.iter().map(|(name, kind, description)| (name.to_string(), json!({ "type": kind, "description": description }))).collect();
            json!({
                "toolName": tool.name,
                "action": format!("cad.{verb}"),
                "domain": tool.domain,
                "verb": verb,
                "description": tool.description,
                "risk": tool.risk,
                "access": match tool.access {
                    Access::Read => "read",
                    Access::Change => "change",
                },
                "requiresScriptAuthorization": false,
                "paramSchema": { "type": "object", "properties": properties },
            })
        })
        .collect();
    json!({
        "platform": super::PLATFORM,
        "displayName": super::DISPLAY_NAME,
        "pluginVersion": env!("CARGO_PKG_VERSION"),
        "cadVersion": photocraft_engine::build_info::long_version(),
        "units": "pixels",
        "supportsDocumentFence": true,
        "operations": operations,
    })
}

/// Run the tool `params` names: `{name | toolName, parameters?, expectedRuntimeDocumentId?}`.
///
/// A request meant for another document than the open one is refused before anything runs, and
/// so is a change while the person is in the middle of something ([`super::busy`]).
pub fn call(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let name = params.get("name").or_else(|| params.get("toolName")).and_then(Value::as_str).ok_or("missing tool `name`")?;
    let tool = find(name).ok_or_else(|| format!("UNSUPPORTED_TOOL: PAULGA Photo has no tool named `{name}`; get-capabilities lists them"))?;
    let empty = json!({});
    let parameters = match params.get("parameters") {
        None | Some(Value::Null) => &empty,
        Some(p) if p.is_object() => p,
        Some(_) => return Err("tool `parameters` must be an object".into()),
    };
    super::publish(app);
    if let (Some(expected), Some(link)) = (params.get("expectedRuntimeDocumentId").and_then(Value::as_str), &app.studio) {
        let open = link.snapshot().document.runtime_id;
        if open != expected {
            return Err(format!("DOCUMENT_FENCE_MISMATCH: the request was for document {expected}, and PAULGA Photo now shows {open}"));
        }
    }
    if tool.access != Access::Read
        && let Some(why) = super::busy(app)
    {
        return Err(format!("CAD_BUSY: {why} in PAULGA Photo; finish or cancel it first"));
    }
    let result = run(app, tool.name, parameters)?;
    if tool.access != Access::Read {
        // The change did not come from this window: say so where the person looks.
        super::say(app, crate::i18n::fmt(tl!("PAULGA Studio ran {tool}"), &[("tool", tool.name)]));
    }
    Ok(result)
}

fn run(app: &mut PhotocraftApp, name: &str, p: &Value) -> Result<Value, String> {
    match name {
        "get-capabilities" => Ok(capabilities()),
        "get-context" => Ok(context(app)),
        "get-selection" => Ok(selection(app)),
        "query-buckets" => Ok(buckets(app)),
        "list-commands" => commands(app, p),
        "capture-view" => capture(app, p),
        "run-command" => run_command(app, p),
        "undo" => undo(app, p),
        "apply-layer-visibility" => layer_visibility(app, p),
        "export-selected-mesh" => super::exchange::export_selected_mesh(app, p),
        "import-mesh-bundle" => super::exchange::import_mesh_bundle(app, p),
        other => Err(format!("UNSUPPORTED_TOOL: {other}")),
    }
}

/// The active document as `document.inspect` reports it; `None` with no document open.
fn inspect(app: &mut PhotocraftApp) -> Option<Map<String, Value>> {
    app.session.active()?;
    match app.session.execute("document.inspect", json!({})) {
        Ok(Value::Object(document)) => Some(document),
        _ => None,
    }
}

/// Every layer of a `document.inspect` tree, top to bottom, with the group that holds it.
fn flat_layers<'a>(layers: &'a [Value], parent: Option<&'a Value>, depth: u64, out: &mut Vec<(&'a Value, Option<&'a Value>, u64)>) {
    for layer in layers {
        out.push((layer, parent, depth));
        if let Some(children) = layer.get("children").and_then(Value::as_array) {
            flat_layers(children, layer.get("id"), depth + 1, out);
        }
    }
}

fn layers_of(document: &Map<String, Value>) -> Vec<(&Value, Option<&Value>, u64)> {
    let mut out = Vec::new();
    flat_layers(document.get("layers").and_then(Value::as_array).map_or(&[], Vec::as_slice), None, 0, &mut out);
    out
}

fn bucket_id(layer: &Value) -> Value {
    layer.get("id").and_then(Value::as_u64).map_or(Value::Null, |id| json!(format!("layer:{id}")))
}

/// Studio's 모델 작업 layer eye and its "only these" focus: each named layer shown or hidden with
/// Layer › Show Layers / Hide Layers, one history step each. A name matches case-insensitively;
/// `id` takes a bucket id (`layer:<id>`).
fn layer_visibility(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let document = inspect(app).ok_or("NO_DOCUMENT: no document is open in PAULGA Photo")?;
    let layers: Vec<(u64, String, bool)> = layers_of(&document)
        .into_iter()
        .filter_map(|(layer, ..)| {
            Some((layer.get("id")?.as_u64()?, layer.get("name")?.as_str()?.to_string(), layer.get("visible") == Some(&json!(true))))
        })
        .collect();
    let mut wanted: Vec<(u64, bool)> = Vec::new();
    if p.get("showAll").and_then(Value::as_bool) == Some(true) {
        wanted.extend(layers.iter().map(|(id, ..)| (*id, true)));
    }
    let mut failed = Vec::new();
    for state in p.get("states").and_then(Value::as_array).into_iter().flatten() {
        let Some(visible) = state.get("visible").and_then(Value::as_bool) else {
            return Err("each of `states` needs a layer `name` (or `id`) and a boolean `visible`".into());
        };
        let by_id = state.get("id").and_then(Value::as_str).and_then(|id| id.strip_prefix("layer:")).and_then(|id| id.parse::<u64>().ok());
        let name = state.get("name").and_then(Value::as_str);
        let found = layers.iter().find(|(id, n, _)| by_id == Some(*id) || name.is_some_and(|name| n.eq_ignore_ascii_case(name)));
        match found {
            Some((id, ..)) => {
                wanted.retain(|(w, _)| w != id);
                wanted.push((*id, visible));
            }
            None => failed.push(json!({ "name": name.unwrap_or_default(), "error": "no such layer" })),
        }
    }
    if wanted.is_empty() && failed.is_empty() {
        return Err("apply-layer-visibility needs `states` [{name, visible}] or `showAll: true`".into());
    }
    let (mut changed, mut unchanged) = (0u32, 0u32);
    for (id, visible) in wanted {
        if layers.iter().any(|(l, _, shown)| *l == id && *shown == visible) {
            unchanged += 1;
            continue;
        }
        let command = if visible { "layer.showLayers" } else { "layer.hideLayers" };
        match app.run(command, json!({ "layer": id })) {
            Ok(_) => changed += 1,
            Err(error) => failed.push(json!({ "id": format!("layer:{id}"), "error": error })),
        }
    }
    Ok(json!({ "changed": changed, "unchanged": unchanged, "failed": failed }))
}

fn context(app: &mut PhotocraftApp) -> Value {
    let session = app.session.execute("session.inspect", json!({})).unwrap_or(Value::Null);
    let runtime_id = app.studio.as_ref().map(|link| link.snapshot().document.runtime_id);
    let mut out = json!({
        "platform": super::PLATFORM,
        "cadVersion": photocraft_engine::build_info::long_version(),
        "documents": session.get("documents").cloned().unwrap_or_else(|| json!([])),
        "activeDocument": session.get("active").cloned().unwrap_or(Value::Null),
        "document": Value::Null,
        "runtimeDocumentId": runtime_id,
        "busy": super::busy(app),
    });
    // No document open (the Home screen): the list is empty and `document` stays null.
    if let Some(document) = inspect(app) {
        let layers = layers_of(&document);
        let mut shown = Map::new();
        for key in ["name", "path", "width", "height", "mode", "depth", "resolution", "revision", "canUndo", "canRedo", "hasSelection", "selectionBounds", "activeLayer"] {
            shown.insert(key.into(), document.get(key).cloned().unwrap_or(Value::Null));
        }
        shown.insert("units".into(), json!("pixels"));
        shown.insert("layerCount".into(), json!(layers.len()));
        shown.insert("selectedLayerCount".into(), json!(layers.iter().filter(|(layer, ..)| layer["selected"] == true).count()));
        out["document"] = Value::Object(shown);
    }
    out
}

fn selection(app: &mut PhotocraftApp) -> Value {
    let Some(document) = inspect(app) else {
        return json!({ "selectedIds": [], "selectedCount": 0, "selectedLayers": [], "activeLayer": Value::Null, "hasPixelSelection": false, "selectionBounds": Value::Null });
    };
    let selected: Vec<Value> = layers_of(&document)
        .into_iter()
        .filter(|(layer, ..)| layer["selected"] == true)
        .map(|(layer, ..)| json!({ "id": layer["id"], "bucketId": bucket_id(layer), "name": layer["name"], "kind": layer["kind"], "visible": layer["visible"] }))
        .collect();
    let ids: Vec<Value> = selected.iter().map(|layer| layer["bucketId"].clone()).collect();
    json!({
        "selectedIds": ids,
        "selectedCount": selected.len(),
        "selectedLayers": selected,
        "activeLayer": document.get("activeLayer").cloned().unwrap_or(Value::Null),
        "hasPixelSelection": document.get("hasSelection").cloned().unwrap_or(json!(false)),
        "selectionBounds": document.get("selectionBounds").cloned().unwrap_or(Value::Null),
    })
}

fn buckets(app: &mut PhotocraftApp) -> Value {
    let document = inspect(app).unwrap_or_default();
    let app = &*app;
    // Each layer's picture as the Layers panel draws it, framed as its Thumbnail Contents says.
    let contents = crate::panels::layers_panel_ui::thumbnail_contents(app);
    let thumbs = app.session.active().map(|st| crate::panels::layers_tree::PhotoThumbs::new(app, &st.doc));
    let out: Vec<Value> = layers_of(&document)
        .into_iter()
        .enumerate()
        .map(|(index, (layer, parent, depth))| {
            let thumbnail = thumbs
                .as_ref()
                .filter(|_| index < THUMBNAIL_LAYERS)
                .zip(layer["id"].as_u64())
                .and_then(|(thumbs, id)| layer_thumbnail(thumbs, id, contents));
            json!({
                "id": bucket_id(layer),
                "name": layer["name"],
                "kind": "layer",
                "visible": layer["visible"] == true,
                "meta": {
                    "layerId": layer["id"], "layerKind": layer["kind"], "opacity": layer["opacity"], "fill": layer["fill"], "blend": layer["blend"],
                    "selected": layer["selected"], "clipped": layer["clipped"], "hasMask": layer["hasMask"], "bounds": layer["bounds"],
                    "parent": parent.and_then(Value::as_u64).map(|id| format!("layer:{id}")), "depth": depth,
                    "thumbnail": thumbnail,
                },
            })
        })
        .collect();
    json!({ "kinds": ["layer"], "count": out.len(), "buckets": out })
}

/// How many layers get a picture in Studio's layer list; past that the list stays light.
const THUMBNAIL_LAYERS: usize = 200;

/// A layer's picture as the Layers panel draws it, as a small PNG data URL for Studio's layer list
/// (a 16-point cell at twice the pixels); none for a layer the panel draws no picture for (a
/// group, an adjustment, text).
fn layer_thumbnail(
    thumbs: &crate::panels::layers_tree::PhotoThumbs<'_>,
    id: u64,
    contents: craft::layers::ui::thumbs::ThumbContents,
) -> Option<String> {
    use craft::layers::ui::thumbs::LayerThumb;
    use craft::ui::primitives::{ThumbRender, ThumbRequest, ThumbSource};
    let key = LayerThumb::pixels(photocraft_doc::LayerId(id), contents);
    let ThumbRender::Ready(image) = thumbs.render(&key, &ThumbRequest::for_cell(16.0, 2.0)) else { return None };
    let [width, height] = image.size;
    let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let png = photocraft_codecs::Image::from_u8(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgba, pixels)
        .and_then(|image| photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()))
        .ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// Why `run-command` does not run command `id` with `params`; `None` when it may.
fn refusal(id: &str, params: &Value) -> Option<&'static str> {
    let lower = id.to_ascii_lowercase();
    if REFUSED.iter().any(|refused| refused.eq_ignore_ascii_case(&lower)) {
        return Some("it plays actions, runs scripts or ends the session");
    }
    if lower.starts_with(PLUGIN_PREFIX) && lower != PLUGIN_LIST {
        return Some("it installs or runs plug-in code");
    }
    // The same through the preferences: the scripts to run on events, the plug-ins to load.
    if lower == "prefs.set" || lower == "prefs.reset" {
        let section = |path: &str| path.split('.').find(|segment| !segment.is_empty()).map(str::to_ascii_lowercase);
        let named = |path: &str| section(path).is_none_or(|section| REFUSED_PREFS.contains(&section.as_str()));
        let direct = params.get("path").and_then(Value::as_str).is_some_and(named);
        let batch = params.get("values").and_then(Value::as_object).is_some_and(|values| values.keys().any(|path| named(path)));
        if direct || batch {
            return Some("it sets which scripts or plug-ins PAULGA Photo runs");
        }
    }
    None
}

fn commands(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let filter = p.get("filter").and_then(Value::as_str).unwrap_or("").trim();
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(60).clamp(1, 1000) as usize;
    let all = app.session.execute("command.list", json!({})).map_err(|e| e.to_string())?;
    let matching: Vec<&Value> = all
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|c| c["id"].as_str().is_some_and(|id| refusal(id, &Value::Null).is_none()))
                .filter(|c| filter.is_empty() || photocraft_engine::commands::list_entry_matches(c, filter))
                .collect()
        })
        .unwrap_or_default();
    let listed: Vec<Value> = matching
        .iter()
        .take(limit)
        .map(|c| json!({ "id": c["id"], "label": c["label"], "params": c["params"], "enabled": c["enabled"], "disabledReason": c["disabledReason"] }))
        .collect();
    Ok(json!({ "count": matching.len(), "returned": listed.len(), "commands": listed }))
}

/// The part of the active document the main canvas shows, in document pixels; `None` when the
/// canvas has not been drawn or shows none of the document.
fn visible_rect(app: &PhotocraftApp, bounds: photocraft_geom::Rect) -> Option<photocraft_geom::Rect> {
    let view = crate::canvas::active_view(app)?;
    let corners = [view.rect.left_top(), view.rect.right_top(), view.rect.left_bottom(), view.rect.right_bottom()].map(|corner| view.to_doc(corner));
    let (x0, x1) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| (lo.min(c.x), hi.max(c.x)));
    let (y0, y1) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| (lo.min(c.y), hi.max(c.y)));
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let edge = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let shown = photocraft_geom::Rect::new(edge(x0.floor()), edge(y0.floor()), edge(x1.ceil()), edge(y1.ceil())).intersect(&bounds);
    (!shown.is_empty()).then_some(shown)
}

fn capture(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let whole = match p.get("mode").and_then(Value::as_str).unwrap_or("current") {
        "current" => false,
        "whole" => true,
        other => return Err(format!("capture-view mode must be `current` or `whole`, not `{other}`")),
    };
    let long_edge = p.get("longEdgePx").and_then(Value::as_f64).unwrap_or(DEFAULT_LONG_EDGE).clamp(*LONG_EDGE.start(), *LONG_EDGE.end());
    let state = app.session.active().ok_or("NO_DOCUMENT: no document is open in PAULGA Photo")?;
    let document = state.doc.clone();
    let bounds = document.bounds();
    // `current` is the part of the document the window shows, unturned; the whole document when
    // the window shows all of it or has not drawn it yet.
    let area = if whole { None } else { visible_rect(app, bounds) }.filter(|area| *area != bounds);
    let source = area.unwrap_or(bounds);
    let scale = (long_edge / f64::from(source.width().max(source.height()).max(1))).min(1.0);
    let size = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    let image = photocraft_compose::render_reduced_rect(&document, source, size(source.width()), size(source.height())).to_rgba8();
    let (width, height) = (image.width, image.height);
    let encoded = photocraft_codecs::Image::from_u8(width, height, photocraft_codecs::ChannelLayout::Rgba, image.pixels)
        .and_then(|image| photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()))
        .map_err(|e| format!("CAPTURE_FAILED: {e}"))?;
    Ok(json!({
        "image": base64::engine::general_purpose::STANDARD.encode(&encoded),
        "mimeType": "image/png",
        "width": width,
        "height": height,
        "mode": if area.is_some() { "current" } else { "whole" },
        "sourceRect": [source.x0, source.y0, source.width(), source.height()],
        "documentSize": [bounds.width(), bounds.height()],
    }))
}

fn run_command(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("command").or_else(|| p.get("id")).and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).ok_or("run-command needs `command`")?;
    let params = match p.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(params) if params.is_object() => params.clone(),
        Some(_) => return Err("run-command `params` must be an object".into()),
    };
    if let Some(why) = refusal(id, &params) {
        return Err(format!("COMMAND_REFUSED: PAULGA Studio does not run {id} through run-command: {why}"));
    }
    // Only the engine's commands: they run with the parameters given and never open a dialog or
    // a file picker, which the window's own commands may.
    if photocraft_engine::commands::find(id).is_none() {
        return Err(format!("COMMAND_UNKNOWN: `{id}` is not a command list-commands lists"));
    }
    // What the menus refuse now, this refuses too (a dialog opened since the busy check).
    if !crate::menus::allows(app, &craft::keymap::CommandRef::new(id)) {
        return Err(format!("CAD_BUSY: {id} is unavailable in PAULGA Photo while a dialog is open"));
    }
    crate::type_box_preview::end_drag(app);
    let previous = std::mem::replace(&mut app.programmatic_playback, true);
    let result = app.run(id, params);
    app.programmatic_playback = previous;
    Ok(json!({ "command": id, "result": result? }))
}

fn undo(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, MAX_UNDO);
    let mut undone = 0;
    while undone < count && app.session.is_enabled("edit.undo") {
        app.run("edit.undo", json!({}))?;
        undone += 1;
    }
    if undone == 0 {
        return Err("NOTHING_TO_UNDO: the document has no change to undo".into());
    }
    Ok(json!({ "undone": undone, "requested": count }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::Link;
    use photocraft_engine::Session;

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({ "width": 64, "height": 32 })).unwrap();
        app
    }

    fn tool(app: &mut PhotocraftApp, name: &str, parameters: Value) -> Result<Value, String> {
        call(app, &json!({ "name": name, "parameters": parameters }))
    }

    fn layer_count(app: &mut PhotocraftApp) -> u64 {
        tool(app, "query-buckets", json!({})).unwrap()["count"].as_u64().unwrap()
    }

    #[test]
    fn every_listed_tool_is_one_the_dispatcher_runs_and_there_is_no_script_tool() {
        let listed: Vec<String> = capabilities()["operations"].as_array().unwrap().iter().map(|o| o["toolName"].as_str().unwrap().to_string()).collect();
        assert_eq!(listed, ["get-capabilities", "get-context", "get-selection", "query-buckets", "list-commands", "capture-view", "run-command", "undo", "apply-layer-visibility", "export-selected-mesh", "import-mesh-bundle"]);
        let mut app = app();
        for name in &listed {
            let error = tool(&mut app, name, json!({})).err().unwrap_or_default();
            assert!(!error.starts_with("UNSUPPORTED_TOOL"), "{name}: {error}");
        }
        for name in ["execute-script", "execute-python"] {
            assert!(tool(&mut app, name, json!({ "code": "1" })).unwrap_err().starts_with("UNSUPPORTED_TOOL"), "{name}");
        }
    }

    #[test]
    fn a_command_studio_runs_adds_a_layer_and_undoes_like_one_the_person_ran() {
        let mut app = app();
        let before = layer_count(&mut app);
        let made = tool(&mut app, "run-command", json!({ "command": "layer.new.layer", "params": {} })).unwrap();
        assert_eq!(made["command"], "layer.new.layer");
        assert_eq!(layer_count(&mut app), before + 1);
        assert_eq!(tool(&mut app, "undo", json!({})).unwrap()["undone"], 1);
        assert_eq!(layer_count(&mut app), before);
        assert!(tool(&mut app, "run-command", json!({ "command": "help.about" })).unwrap_err().starts_with("COMMAND_UNKNOWN"), "a window command may open a dialog");
    }

    #[test]
    fn mesh_bundle_round_trips_canvas_and_imports_cad_curves() {
        let mut src = app();
        let exported = tool(&mut src, "export-selected-mesh", json!({})).unwrap();
        assert_eq!(exported["exportedCount"], 1);
        let bundle = exported["bundle"].clone();
        assert!(bundle["meshes"][0]["material"]["textureDataUrl"].as_str().unwrap().starts_with("data:image/png;base64,"));

        let mut dst = app();
        let before = layer_count(&mut dst);
        let imported = tool(&mut dst, "import-mesh-bundle", json!({ "bundle": bundle })).unwrap();
        assert_eq!(imported["importedCount"], 1);
        assert!(layer_count(&mut dst) > before);

        // Curves from AutoCAD / PAULGA CAD / Vector import as an embedded SVG Smart Object layer.
        let cad_bundle = json!({
            "schemaVersion": "3.1",
            "units": "mm",
            "curves": [{ "id": "c1", "layerName": "Walls", "closed": true, "points": [0,0,0, 100,0,0, 100,50,0, 0,50,0] }],
            "meshes": []
        });
        let before2 = layer_count(&mut dst);
        let imp2 = tool(&mut dst, "import-mesh-bundle", json!({ "bundle": cad_bundle })).unwrap();
        assert_eq!(imp2["importedCount"], 1);
        assert!(layer_count(&mut dst) > before2);
    }

    #[test]
    fn run_command_refuses_code_actions_and_quitting_under_any_spelling() {
        let mut app = app();
        let before = layer_count(&mut app);
        let refused = [
            ("actions.play", json!({ "action": "x" })),
            ("ACTIONS.PLAY", json!({})),
            ("actions.load", json!({ "path": "C:/x.atn" })),
            ("file.automate.batch", json!({})),
            ("file.scripts.browse", json!({ "path": "C:/x.jsx" })),
            ("file.scripts.scriptEventsManager", json!({})),
            ("plugin.install", json!({ "path": "C:/x.wasm" })),
            ("plugin.run", json!({ "id": "x" })),
            ("plugin.filter.org.example", json!({})),
            ("file.exit", json!({})),
            ("studio.disconnect", json!({})),
            ("prefs.set", json!({ "path": "scriptEvents.enabled", "value": true })),
            ("prefs.set", json!({ "values": { "plugIns.folder": "C:/x" } })),
        ];
        for (command, params) in refused {
            let error = tool(&mut app, "run-command", json!({ "command": command, "params": params })).unwrap_err();
            assert!(error.starts_with("COMMAND_REFUSED"), "{command}: {error}");
        }
        assert_eq!(layer_count(&mut app), before);
        let listed = tool(&mut app, "list-commands", json!({ "limit": 1000 })).unwrap();
        let ids: Vec<&str> = listed["commands"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"layer.new.layer") && ids.contains(&"plugin.list"));
        for id in ["actions.play", "plugin.install", "file.scripts.browse"] {
            assert!(photocraft_engine::commands::find(id).is_some(), "{id} is a command the refusal guards");
            assert!(!ids.contains(&id), "{id} is not offered");
        }
        // A preference that names no script or plug-in is an ordinary command.
        assert_eq!(refusal("prefs.set", &json!({ "path": "general.beepWhenDone", "value": true })), None);
    }

    #[test]
    fn the_command_list_is_filtered_and_limited() {
        let mut app = app();
        let blur = tool(&mut app, "list-commands", json!({ "filter": "gaussian blur" })).unwrap();
        assert_eq!(blur["commands"][0]["id"], "filter.blur.gaussianBlur");
        assert!(blur["commands"][0]["params"].is_string());
        let few = tool(&mut app, "list-commands", json!({ "limit": 3 })).unwrap();
        assert_eq!(few["returned"], 3);
        assert!(few["count"].as_u64().unwrap() > 3);
    }

    #[test]
    fn a_request_for_another_document_changes_nothing() {
        let link = Link::new(true);
        let mut app = app();
        app.studio = Some(link.clone());
        crate::studio::publish(&app);
        let open = link.snapshot().document.runtime_id;
        let before = layer_count(&mut app);
        let add = |expected: &str| json!({ "name": "run-command", "expectedRuntimeDocumentId": expected, "parameters": { "command": "layer.new.layer" } });
        assert!(call(&mut app, &add("someone-else:1")).unwrap_err().starts_with("DOCUMENT_FENCE_MISMATCH"));
        assert_eq!(layer_count(&mut app), before);
        call(&mut app, &add(&open)).unwrap();
        assert_eq!(layer_count(&mut app), before + 1);
        // Another document became the open one: the id the request was written for no longer fits.
        app.run("file.new", json!({ "width": 8, "height": 8 })).unwrap();
        assert!(call(&mut app, &add(&open)).unwrap_err().starts_with("DOCUMENT_FENCE_MISMATCH"));
    }

    #[test]
    fn a_change_waits_while_the_person_is_inside_a_dialog_and_reads_do_not() {
        let mut app = app();
        let ctx = app.ctx().clone();
        let before = layer_count(&mut app);
        crate::menus::invoke(&mut app, &ctx, "help.about", json!({})).unwrap();
        assert!(app.dialog_open());
        let error = tool(&mut app, "run-command", json!({ "command": "layer.new.layer" })).unwrap_err();
        assert!(error.starts_with("CAD_BUSY"), "{error}");
        assert!(tool(&mut app, "undo", json!({})).unwrap_err().starts_with("CAD_BUSY"));
        assert!(tool(&mut app, "get-context", json!({})).is_ok());
        assert_eq!(layer_count(&mut app), before);
        assert!(app.dialog_open(), "the person's dialog is still open");
    }

    #[test]
    fn the_context_names_the_open_document_and_is_empty_without_one() {
        let mut none = PhotocraftApp::new(Session::new(), crate::Services::default());
        let empty = tool(&mut none, "get-context", json!({})).unwrap();
        assert_eq!(empty["document"], Value::Null);
        assert_eq!(empty["documents"], json!([]));
        assert_eq!(tool(&mut none, "query-buckets", json!({})).unwrap()["count"], 0);
        assert_eq!(tool(&mut none, "get-selection", json!({})).unwrap()["selectedCount"], 0);
        assert!(tool(&mut none, "capture-view", json!({})).unwrap_err().starts_with("NO_DOCUMENT"));
        let mut app = app();
        let context = tool(&mut app, "get-context", json!({})).unwrap();
        assert_eq!(context["documents"].as_array().unwrap().len(), 1);
        let document = &context["document"];
        assert_eq!((document["width"].clone(), document["height"].clone()), (json!(64), json!(32)));
        assert!(document["mode"].is_string() && document["name"].is_string());
        assert_eq!(document["layerCount"], layer_count(&mut app));
        assert!(document.get("layers").is_none(), "layers come from query-buckets");
    }

    #[test]
    fn the_selection_names_the_selected_layers_and_a_pixel_selection() {
        let mut app = app();
        let made = tool(&mut app, "run-command", json!({ "command": "layer.new.layer" })).unwrap();
        let layer = made["result"]["layer"].as_u64().unwrap();
        let read = tool(&mut app, "get-selection", json!({})).unwrap();
        assert_eq!(read["selectedIds"], json!([format!("layer:{layer}")]));
        assert_eq!(read["selectedLayers"][0]["id"], layer);
        assert_eq!(read["hasPixelSelection"], false);
        tool(&mut app, "run-command", json!({ "command": "select.rect", "params": { "x": 2, "y": 2, "width": 10, "height": 5 } })).unwrap();
        let read = tool(&mut app, "get-selection", json!({})).unwrap();
        assert_eq!(read["hasPixelSelection"], true);
        assert_eq!(read["selectionBounds"], json!([2, 2, 10, 5]));
        // The bucket with that id is the layer, in a group when it is in one.
        tool(&mut app, "run-command", json!({ "command": "layer.groupLayers" })).unwrap();
        let listed = tool(&mut app, "query-buckets", json!({})).unwrap();
        let bucket = listed["buckets"].as_array().unwrap().iter().find(|b| b["id"] == format!("layer:{layer}")).unwrap().clone();
        assert_eq!((bucket["kind"].clone(), bucket["visible"].clone(), bucket["meta"]["depth"].clone()), (json!("layer"), json!(true), json!(1)));
        assert!(bucket["meta"]["parent"].as_str().is_some_and(|parent| parent.starts_with("layer:")));
        // A pixel layer comes with its picture; the group around it, which the panel draws no
        // picture for, without one.
        assert!(bucket["meta"]["thumbnail"].as_str().is_some_and(|url| url.starts_with("data:image/png;base64,iVBOR")), "{bucket}");
        let group = listed["buckets"].as_array().unwrap().iter().find(|b| bucket["meta"]["parent"] == b["id"]).unwrap();
        assert_eq!(group["meta"]["thumbnail"], Value::Null);
    }

    #[test]
    fn a_capture_is_a_png_no_larger_than_asked_or_than_the_document() {
        let mut app = app();
        let shot = tool(&mut app, "capture-view", json!({ "mode": "whole", "longEdgePx": 256 })).unwrap();
        assert!(shot["image"].as_str().unwrap().starts_with("iVBOR"), "base64 of a PNG");
        assert_eq!((shot["width"].clone(), shot["height"].clone(), shot["mimeType"].clone()), (json!(64), json!(32), json!("image/png")));
        let mut large = PhotocraftApp::new(Session::new(), crate::Services::default());
        large.run("file.new", json!({ "width": 1000, "height": 500 })).unwrap();
        let shot = tool(&mut large, "capture-view", json!({ "longEdgePx": 1 })).unwrap();
        assert_eq!(shot["width"], 256, "the long edge is at least 256");
        assert!(shot["height"].as_u64().unwrap() < 256);
        assert_eq!(shot["mode"], "whole", "no window has drawn the document, so the whole of it is shown");
        assert!(tool(&mut large, "capture-view", json!({ "mode": "sideways" })).is_err());
    }
}
