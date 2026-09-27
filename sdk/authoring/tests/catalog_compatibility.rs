use cranpose_plugin_authoring::Catalog;
use sha2::{Digest, Sha256};

#[path = "../examples/support/mod.rs"]
mod support;

#[test]
fn complete_catalogs_match_pre_index_baseline() {
    let baseline: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/measurements/catalog-0.4.1/baseline.json"
    ))
    .expect("retained baseline");
    assert_eq!(
        baseline["cases"].as_array().expect("cases").len(),
        support::CASES.len()
    );
    for ((functions, unicode, single_line), expected) in support::CASES
        .into_iter()
        .zip(baseline["cases"].as_array().expect("baseline cases"))
    {
        let source = support::fixture(functions, unicode, single_line);
        assert_eq!(expected["sourceBytes"], source.len());
        let catalog = Catalog::parse(&source).expect("catalog");
        // Covers the schema, every function/call/literal range in both encodings,
        // call ends, literal identity, value and type. Timing is not asserted.
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&catalog).expect("serialize"))
        );
        assert_eq!(expected["catalogSha256"], digest);
    }
}
