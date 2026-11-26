//! Bidirectional PAULGA Send Mode mesh-bundle exchange (`export-selected-mesh`, `import-mesh-bundle`).
//!
//! Exports the active Photo document as a textured 2D image quad mesh (PNG data URL + mm quad) so
//! Rhino, SketchUp, Blender, Workshare and PAULGA Vector can place it directly, and imports curves,
//! meshes and images from any CAD/Vector host as embedded Smart Object layers via `place_bytes`.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const SCHEMA_VERSION: &str = "3.1";
const MAX_EXPORT_EDGE: f64 = 2048.0;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn export_selected_mesh(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let state = app.session.active().ok_or("NO_DOCUMENT: no document is open in PAULGA Photo")?;
    let document = state.doc.clone();
    let doc_name = state
        .path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled.psd".into());
    let bounds = document.bounds();
    // A pixel selection sends what it encloses (its bounding box), in its place on the canvas;
    // no selection, or `"scope": "all"`, sends the whole canvas.
    let whole = p.get("scope").and_then(Value::as_str) == Some("all");
    let area = if whole { None } else { document.selection.as_ref().map(|s| s.content_bounds().intersect(&bounds)) }
        .filter(|r| !r.is_empty() && *r != bounds);
    let source = area.unwrap_or(bounds);
    let dpi = f64::from(document.resolution_dpi).max(72.0);
    let mm = |px: i64| (px as f64 / dpi) * 25.4;
    let width_mm = mm(i64::from(source.width()));
    let height_mm = mm(i64::from(source.height()));
    let x0_mm = mm(i64::from(source.x0) - i64::from(bounds.x0));
    // The bundle is Y-up: the area's bottom edge, from the canvas's bottom edge.
    let y0_mm = mm(i64::from(bounds.y1) - i64::from(source.y1));

    let scale = (MAX_EXPORT_EDGE / f64::from(source.width().max(source.height()).max(1))).min(1.0);
    let out_w = ((f64::from(source.width()) * scale).round() as u32).max(1);
    let out_h = ((f64::from(source.height()) * scale).round() as u32).max(1);
    let image = photocraft_compose::render_reduced_rect(&document, source, out_w, out_h).to_rgba8();
    let png = photocraft_codecs::Image::from_u8(image.width, image.height, photocraft_codecs::ChannelLayout::Rgba, image.pixels)
        .and_then(|img| photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()))
        .map_err(|e| format!("EXPORT_ENCODE_FAILED: {e}"))?;
    let data_url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));

    let mesh = json!({
        "id": if area.is_some() { "mesh_photo_selection" } else { "mesh_photo_canvas" },
        "sourceObjectIds": [if area.is_some() { "selection" } else { "canvas" }],
        "sourceObjectNames": [doc_name],
        "layerName": "PAULGA Photo",
        "vertices": [
            x0_mm, y0_mm, 0.0,
            x0_mm + width_mm, y0_mm, 0.0,
            x0_mm + width_mm, y0_mm + height_mm, 0.0,
            x0_mm, y0_mm + height_mm, 0.0,
        ],
        "indices": [0, 1, 2, 0, 2, 3],
        "uvs": [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
        "material": {
            "name": doc_name,
            "diffuseColor": [255, 255, 255, 255],
            "textureDataUrl": data_url,
        },
    });
    let stats = json!({
        "meshCount": 1,
        "objectCount": 1,
        "vertexCount": 4,
        "faceCount": 2,
        "triangleCount": 2,
        "curveCount": 0,
        "pointCount": 0,
        "textCount": 0,
    });
    let bundle_bounds = json!({
        "min": { "x": x0_mm, "y": y0_mm, "z": 0.0 },
        "max": { "x": x0_mm + width_mm, "y": y0_mm + height_mm, "z": 0.0 },
    });

    let bundle = json!({
        "schemaVersion": SCHEMA_VERSION,
        "units": "mm",
        "source": {
            "platform": super::PLATFORM,
            "documentName": doc_name,
        },
        "selectedCount": 1,
        "exportedCount": 1,
        "meshes": [mesh],
        "curves": [],
        "points": [],
        "instances": [],
        "definitions": {},
        "texts": [],
        "hatches": [],
        "stats": stats,
        "bounds": bundle_bounds,
    });

    let mut result = json!({
        "selectedElementCount": 1,
        "selectedCount": 1,
        "exportedCount": 1,
        "stats": stats,
        "bounds": bundle_bounds,
    });

    if let Some(path) = p.get("bundleFilePath").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        let bytes = serde_json::to_vec(&bundle).map_err(|e| format!("bundle serialize failed: {e}"))?;
        if let Some(parent) = Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, &bytes).map_err(|e| format!("failed to write bundle file {path}: {e}"))?;
        result["bundleFilePath"] = json!(path);
        result["bundleSizeBytes"] = json!(bytes.len());
    } else {
        result["bundle"] = bundle;
    }

    Ok(result)
}

fn load_bundles(p: &Value) -> Result<Vec<Value>, String> {
    if let Some(bundle) = p.get("bundle").filter(|v| v.is_object()) {
        return Ok(vec![bundle.clone()]);
    }
    let mut paths: Vec<String> = Vec::new();
    if let Some(path) = p.get("bundleFilePath").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        paths.push(path.to_string());
    }
    if let Some(arr) = p.get("bundleFilePaths").and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str().filter(|s| !s.trim().is_empty()) {
                paths.push(s.to_string());
            }
        }
    }
    if paths.is_empty() {
        return Err("import-mesh-bundle needs `bundle` or `bundleFilePath`".into());
    }
    let mut bundles = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("failed to read bundle file {path}: {e}"))?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| format!("invalid bundle JSON in {path}: {e}"))?;
        bundles.push(parsed);
    }
    Ok(bundles)
}

fn parse_points_2d(val: &Value) -> Vec<[f64; 2]> {
    let Some(arr) = val.as_array() else { return Vec::new() };
    if arr.is_empty() {
        return Vec::new();
    }
    if arr[0].is_number() {
        return arr.chunks_exact(3).filter_map(|c| Some([c[0].as_f64()?, c[1].as_f64()?])).collect();
    }
    arr.iter()
        .filter_map(|item| {
            if let Some(a) = item.as_array() {
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
            } else if let Some(o) = item.as_object() {
                Some([o.get("x")?.as_f64()?, o.get("y")?.as_f64()?])
            } else {
                None
            }
        })
        .collect()
}

pub fn import_mesh_bundle(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("NO_DOCUMENT: no document is open in PAULGA Photo".into());
    }
    let bundles = load_bundles(p)?;

    let mut polylines: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
    let mut png_blobs: Vec<Vec<u8>> = Vec::new();

    for bundle in &bundles {
        if let Some(curves) = bundle.get("curves").and_then(Value::as_array) {
            for curve in curves {
                let pts = curve.get("points").map(parse_points_2d).unwrap_or_default();
                let closed = curve.get("closed").and_then(Value::as_bool).unwrap_or(false);
                if pts.len() >= 2 {
                    polylines.push((pts, closed));
                }
            }
        }
        if let Some(meshes) = bundle.get("meshes").and_then(Value::as_array) {
            for mesh in meshes {
                if let Some(url) = mesh.pointer("/material/textureDataUrl").and_then(Value::as_str)
                    && let Some(b64) = url.strip_prefix("data:image/png;base64,")
                    && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64)
                {
                    png_blobs.push(bytes);
                    continue;
                }
                let Some(verts_arr) = mesh.get("vertices").and_then(Value::as_array) else { continue };
                let Some(idx_arr) = mesh.get("indices").and_then(Value::as_array) else { continue };
                let verts: Vec<[f64; 2]> = verts_arr.chunks_exact(3).filter_map(|c| Some([c[0].as_f64()?, c[1].as_f64()?])).collect();
                let indices: Vec<usize> = idx_arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect();
                for tri in indices.chunks_exact(3).take(5_000) {
                    if let (Some(&a), Some(&b), Some(&c)) = (verts.get(tri[0]), verts.get(tri[1]), verts.get(tri[2])) {
                        polylines.push((vec![a, b, c], true));
                    }
                }
            }
        }
    }

    // Every image is checked before anything is placed, so a broken one leaves the document as it was.
    if let Some(bad) = png_blobs.iter().position(|png| !png.starts_with(PNG_SIGNATURE)) {
        return Err(format!("INVALID_IMAGE: image {bad} of the bundle is not a PNG"));
    }

    let mut imported_count = 0usize;

    if !polylines.is_empty() {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (pts, _) in &polylines {
            for &[x, y] in pts {
                min_x = min_x.min(x);
                min_y = min_y.min(-y);
                max_x = max_x.max(x);
                max_y = max_y.max(-y);
            }
        }
        let span_x = (max_x - min_x).max(1.0);
        let span_y = (max_y - min_y).max(1.0);
        let pad = span_x.max(span_y) * 0.02;
        let vb_w = span_x + pad * 2.0;
        let vb_h = span_y + pad * 2.0;
        let stroke = (vb_w.max(vb_h) / 500.0).max(0.5);
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="{h:.0}" viewBox="0 0 {vb_w:.2} {vb_h:.2}">"#,
            h = (1024.0 * vb_h / vb_w).clamp(64.0, 2048.0),
        );
        for (pts, closed) in &polylines {
            let mut d = String::new();
            for (idx, &[x, y]) in pts.iter().enumerate() {
                let px = x - min_x + pad;
                let py = -y - min_y + pad;
                let cmd = if idx == 0 { 'M' } else { 'L' };
                let _ = write!(d, "{cmd}{px:.2} {py:.2} ");
            }
            if *closed {
                d.push('Z');
            }
            let _ = write!(svg, r##"<path d="{d}" fill="none" stroke="#111111" stroke-width="{stroke:.2}"/>"##);
        }
        svg.push_str("</svg>");
        photocraft_engine::file_cmds::place_bytes(
            &mut app.session,
            "studio-import.svg",
            svg.into_bytes(),
            None,
            &json!({ "smartObject": true }),
        )
        .map_err(|e| e.to_string())?;
        imported_count += polylines.len();
    }

    for (idx, png) in png_blobs.into_iter().enumerate() {
        photocraft_engine::file_cmds::place_bytes(
            &mut app.session,
            &format!("studio-import-{idx}.png"),
            png,
            None,
            &json!({}),
        )
        .map_err(|e| e.to_string())?;
        imported_count += 1;
    }

    Ok(json!({
        "importedCount": imported_count,
        "createdCount": imported_count,
    }))
}
