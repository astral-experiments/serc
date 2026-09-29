use crate::common;

#[test]
fn postponed_annotations_keep_surrogates_distinct_from_replacement_characters() {
    common::assert_executes(
        r#"from __future__ import annotations
value: '\ufffd\ud800' | None
raw: r'\ud800' '\udfff'
assert __annotations__['value'] == "'�\\ud800' | None"
assert __annotations__['raw'] == "'\\\\ud800\\udfff'"
print('ok')
"#,
        "ok\n",
    );
}
