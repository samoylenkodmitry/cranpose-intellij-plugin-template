//! Shared geometry-aware interactions for native UI regression tools.
use anyhow::{Context, Result};
use cranpose_ide_host::protocol::Packet;
use serde_json::{Value, json};
use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn verify_text(host: &crate::hot_smoke::Host, expected: &str) -> Result<Value> {
    verify_view(host, expected, |view| text_node(view, expected).is_some())
}

pub(crate) fn text_node<'a>(view: &'a Value, expected: &str) -> Option<&'a Value> {
    view["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["text"] == expected)
}

pub(crate) fn verify_view(
    host: &crate::hot_smoke::Host,
    description: &str,
    matches: impl Fn(&Value) -> bool,
) -> Result<Value> {
    static REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let request = REQUEST.fetch_add(1, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut next = Instant::now();
    let mut last = Value::Null;
    while Instant::now() < deadline {
        if Instant::now() >= next {
            host.send(Packet::message(
                "cranpose.inspector.v2.request",
                &request.to_string(),
            ))?;
            next = Instant::now() + Duration::from_millis(20);
        }
        for response in host.messages.try_iter() {
            let (channel, payload, _) = response?;
            if channel == "cranpose.inspector.v2.snapshot"
                && payload["requestId"].as_u64() == Some(request)
            {
                if matches(&payload) {
                    return Ok(payload);
                }
                last = payload;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    let labels: Vec<_> = last["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["text"].as_str().is_some_and(|text| !text.is_empty()))
        .take(60)
        .map(|node| json!({"text":node["text"],"x":node["x"],"y":node["y"]}))
        .collect();
    anyhow::bail!("Inspector UI did not display {description:?}; last labels: {labels:?}")
}

pub(crate) fn center(node: &Value) -> (f32, f32) {
    let number = |key| node[key].as_f64().unwrap_or_default() as f32;
    (
        number("x") + number("width") / 2.0,
        number("y") + number("height") / 2.0,
    )
}

pub(crate) fn click(host: &crate::hot_smoke::Host, view: &Value, text: &str) -> Result<()> {
    // Large wheel gestures can leave overscroll animation moving a matching row.
    // A semantic match alone is not a stable hit target, especially in debug builds.
    let position = std::cell::Cell::new(center(
        text_node(view, text).context("Missing inspector control")?,
    ));
    let stable_since = std::cell::Cell::new(Instant::now());
    let settled = verify_view(host, &format!("stable bounds for {text}"), |view| {
        let Some(node) = text_node(view, text) else {
            stable_since.set(Instant::now());
            return false;
        };
        let current = center(node);
        let previous = position.get();
        if (current.0 - previous.0).abs() > 0.25 || (current.1 - previous.1).abs() > 0.25 {
            position.set(current);
            stable_since.set(Instant::now());
        }
        stable_since.get().elapsed() >= Duration::from_millis(100)
    })?;
    let (x, y) = center(text_node(&settled, text).context("Settled inspector control")?);
    eprintln!("Click {text:?} at {x}, {y}");
    for kind in [3, 4] {
        host.send(Packet::new(kind).int(0).float(x).float(y))?;
    }
    Ok(())
}
