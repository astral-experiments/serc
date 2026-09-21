#![cfg(target_os = "linux")]

use crate::common;

#[test]
fn matches_cpython_neutral_bootstrap_frozenset_identity() {
    // Cover both importlib bootstraps, the encodings package, and aliases retained
    // by the complete module dictionary. No caller imports these modules here.
    for member in [
        "blocking_on",
        "location",
        "source_mtime",
        "norm_encoding",
        "us",
        "iso8859_1",
    ] {
        let source = format!(
            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
             def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
        );
        common::assert_matches_cpython_marshal(&source, "neutral_bootstrap_frozensets.py");
    }
}

#[test]
fn matches_cpython_nested_class_name_retained_by_bootstrap_code() {
    let source = "class _WeakValueDictionary:\n    def __init__(self):\n        class KeyedRef:\n            pass\n        self._KeyedRef = KeyedRef\n";
    common::assert_matches_cpython_marshal(source, "neutral_bootstrap_code_owner.py");
}
