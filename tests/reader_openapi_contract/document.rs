use std::{collections::BTreeSet, fs, path::PathBuf};

use serde_json::Value;

pub(crate) const ENTRY_PATH: &str = "/api/v1/entries/{entryId}";
pub(crate) const ENTRY_STATE_PATH: &str = "/api/v1/entries/{entryId}/state";
const OPENAPI_PATH: &str = "docs/openapi/reader-v1.json";

pub(crate) fn load_openapi() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(OPENAPI_PATH);
    let bytes = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reader OpenAPI artifact {} must exist: {error}",
            path.display()
        )
    });
    serde_json::from_slice(&bytes).expect("reader OpenAPI artifact should contain valid JSON")
}

pub(crate) fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                operations.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    operations
}

pub(crate) fn assert_operation_statuses(
    document: &Value,
    path: &str,
    method: &str,
    expected: &[u16],
) {
    let statuses = document["paths"][path][method]["responses"]
        .as_object()
        .expect("operation responses should be an object")
        .keys()
        .map(|status| {
            status
                .parse::<u16>()
                .expect("response status should be numeric")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(statuses, expected.iter().copied().collect());
}

pub(crate) fn parameter_names(document: &Value, path: &str, method: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for value in [
        document["paths"][path]["parameters"].as_array(),
        document["paths"][path][method]["parameters"].as_array(),
    ]
    .into_iter()
    .flatten()
    .flatten()
    {
        let parameter = resolve_ref(document, value);
        names.insert(
            parameter["name"]
                .as_str()
                .expect("parameter name should be a string")
                .to_owned(),
        );
    }
    names
}

pub(crate) fn assert_required_fields(document: &Value, schema: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema]["required"]
        .as_array()
        .expect("schema required should be an array")
        .iter()
        .map(|value| value.as_str().expect("required field should be a string"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

pub(crate) fn assert_schema_properties(document: &Value, schema: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema]["properties"]
        .as_object()
        .expect("schema properties should be an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

pub(crate) fn assert_all_local_refs_resolve(document: &Value) {
    visit_local_refs(document, document, "$document");
}

fn visit_local_refs(document: &Value, value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let reference = reference.as_str().expect("$ref should be a string");
                assert!(
                    reference.starts_with("#/"),
                    "external ref at {path}: {reference}"
                );
                let pointer = reference
                    .strip_prefix('#')
                    .expect("local ref should start with #");
                assert!(
                    document.pointer(pointer).is_some(),
                    "unresolved local ref at {path}: {reference}"
                );
            }
            for (key, child) in object {
                visit_local_refs(document, child, &format!("{path}.{key}"));
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                visit_local_refs(document, child, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}

pub(crate) fn resolve_ref<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .expect("only local OpenAPI refs are supported");
        return document
            .pointer(pointer)
            .unwrap_or_else(|| panic!("OpenAPI ref should resolve: {reference}"));
    }
    value
}
