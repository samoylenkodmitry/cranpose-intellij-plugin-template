//! Repeatable source-catalog measurements, without the IDE or renderer.
use anyhow::{Context, Result, ensure};
use cranpose_plugin_authoring::Catalog;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};
mod support;

fn main() -> Result<()> {
    let samples = std::env::args()
        .nth(1)
        .map_or(Ok(7), |s| s.parse::<usize>())?;
    ensure!((1..=100).contains(&samples), "Samples must be 1–100");
    let mut cases = vec![];
    for (functions, unicode, single_line) in support::CASES {
        let source = support::fixture(functions, unicode, single_line);
        let expected = Catalog::parse(&source).context("warmup")?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&expected)?));
        let mut milliseconds = vec![];
        for _ in 0..samples {
            let start = Instant::now();
            let result = Catalog::parse(black_box(&source))?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            ensure!(result == expected, "Catalog changed during measurement");
            black_box(result);
            milliseconds.push(elapsed);
        }
        let mut sorted = milliseconds.clone();
        sorted.sort_by(f64::total_cmp);
        cases.push(json!({
            "functions": functions,
            "unicode": unicode,
            "singleLineBodies": single_line,
            "sourceBytes": source.len(),
            "calls": expected.calls.len(),
            "literals": expected.literals.len(),
            "catalogSha256": digest,
            "milliseconds": milliseconds,
            "medianMs": sorted[sorted.len()/2],
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"samples": samples, "cases": cases}))?
    );
    Ok(())
}
