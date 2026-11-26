//! The PAULGA Studio panel: how the connection is doing, and the switch that turns it on and off.
//! Everything it does goes through `studio.connect` and `studio.disconnect`.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::style::TypeRole;
use serde_json::json;

use super::Phase;
use crate::PhotocraftApp;

/// One name and value line.
fn row(ui: &mut egui::Ui, name: &str, value: &str) {
    Row::new().show(ui, |ui, _| {
        Text::new(name.to_string()).shade(Shade::Muted).show(ui);
        Text::new(value.to_string()).show(ui);
    });
}

/// The panel body.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(link) = app.studio.clone() else {
        Text::new(tl!(super::UNAVAILABLE).to_string()).shade(Shade::Muted).show(ui);
        return;
    };
    let s = link.snapshot();
    // Words carry the state; the tone only repeats it.
    let state = Text::new(tl!(s.phase.label()).to_string()).role(TypeRole::BodyStrong);
    match s.phase {
        Phase::Connected => state.show(ui),
        Phase::Refused => state.tone(Tone::Warn).show(ui),
        _ => state.shade(Shade::Muted).show(ui),
    };
    if let Some(note) = &s.note {
        Text::new(tl!(note).to_string()).shade(Shade::Muted).show(ui);
    }
    let mut act = None;
    Row::new().show(ui, |ui, _| {
        let (label, command) = if s.wanted { (tl!("Disconnect"), "studio.disconnect") } else { (tl!("Connect"), "studio.connect") };
        if Button::new(label.to_string()).show(ui).clicked() {
            act = Some(command);
        }
        if s.wanted && matches!(s.phase, Phase::Searching | Phase::Refused) && Button::new(tl!("Try Again").to_string()).show(ui).clicked() {
            act = Some("studio.connect");
        }
    });
    ui.separator();
    row(ui, tl!("Document"), s.document.name.as_deref().unwrap_or(tl!("None")));
    if let Some(version) = &s.studio_version {
        row(ui, tl!("Studio version"), version);
    }
    if let Some(port) = s.port {
        row(ui, tl!("Port"), &port.to_string());
    }
    row(ui, tl!("Tools run"), &s.tools_run.to_string());
    if let Some(tool) = &s.last_tool {
        row(ui, tl!("Last tool"), tool);
    }
    Text::new(tl!("PAULGA Studio on this computer can read the open document and run PAULGA Photo commands in it. Each change is an undo step.").to_string())
        .role(TypeRole::Caption)
        .shade(Shade::Muted)
        .show(ui);
    if let Some(command) = act
        && let Err(error) = app.run(command, json!({}))
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}
