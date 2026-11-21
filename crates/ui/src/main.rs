//! remote-ui (milestone 1): bring up a Slint window, connect to remote-core over the Core-API WS,
//! and show the entity list. The tokio runtime runs on its own thread and pushes updates into the
//! Slint event loop via `invoke_from_event_loop`. See docs/plan.md.

use serde_json::Value;
use slint::{ModelRc, VecModel};
use tracing_subscriber::EnvFilter;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let app = AppWindow::new()?;
    let weak = app.as_weak();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            if let Err(e) = run(weak).await {
                tracing::error!("core loop ended: {e:#}");
            }
        });
    });

    app.run()?;
    Ok(())
}

async fn run(weak: slint::Weak<AppWindow>) -> anyhow::Result<()> {
    let cfg = coreapi::Config::from_env()?;
    set_status(&weak, format!("connecting {}…", cfg.url));

    let (core, mut events) = coreapi::connect(cfg).await?;
    set_status(&weak, "connected".to_string());

    let resp = core.get_entities().await?;
    let rows = parse_entities(&resp.data);
    let n = rows.len();
    push_entities(&weak, rows);
    set_status(&weak, format!("connected — {n} entities"));

    // TODO (milestone 2): react to entity_change / battery_status / wifi_change and update the model.
    while let Some(ev) = events.recv().await {
        tracing::debug!(msg = %ev.msg, cat = ?ev.cat, "event");
    }
    Ok(())
}

fn set_status(weak: &slint::Weak<AppWindow>, s: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            app.set_status(s.into());
        }
    });
}

fn push_entities(weak: &slint::Weak<AppWindow>, rows: Vec<(String, String)>) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            let model: Vec<EntityRow> = rows
                .into_iter()
                .map(|(name, kind)| EntityRow { name: name.into(), kind: kind.into() })
                .collect();
            app.set_entities(ModelRc::new(VecModel::from(model)));
        }
    });
}

fn parse_entities(data: &Value) -> Vec<(String, String)> {
    let arr = data
        .get("entities")
        .and_then(Value::as_array)
        .or_else(|| data.as_array())
        .cloned()
        .unwrap_or_default();
    arr.iter()
        .map(|e| {
            let kind = e
                .get("entity_type")
                .or_else(|| e.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (entity_name(e), kind)
        })
        .collect()
}

/// `name` may be a plain string or a `LanguageText` map (`{"en": "..."}`); fall back to the id.
fn entity_name(e: &Value) -> String {
    match e.get("name") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => m
            .get("en")
            .or_else(|| m.values().next())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => e.get("entity_id").and_then(Value::as_str).unwrap_or("?").to_string(),
    }
}
