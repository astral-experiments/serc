use crate::common;

#[test]
fn borrows_independently_reachable_for_cleanup_after_folded_false() {
    for (definition, action) in [("def", "emit(item)"), ("async def", "await emit(item)")] {
        let source = format!(
            "{definition} f(items, emit):\n    count = 0\n    other = 1\n    for item in items:\n        if item is None:\n            other += 1\n            continue\n        if False:\n            emit(item)\n            continue\n        {action}\n        count += 1\n    emit(count, other)\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_false_for_cleanup.py");
    }
}

#[test]
fn preserves_folded_false_fallthrough_borrow_barriers() {
    for definition in ["def", "async def"] {
        for tail in ["emit(value)", "return emit(value)", "return value"] {
            let source = format!(
                "{definition} f(value, emit):\n    if False:\n        discard()\n    {tail}\n"
            );
            common::assert_matches_cpython_marshal(&source, "folded_false_fallthrough.py");
        }
    }
}

#[test]
fn preserves_for_cleanup_without_a_folded_branch() {
    for (definition, action) in [("def", "emit(item)"), ("async def", "await emit(item)")] {
        let source = format!(
            "{definition} f(items, emit):\n    count = 0\n    other = 1\n    for item in items:\n        {action}\n        count += 1\n    emit(count, other)\n"
        );
        common::assert_matches_cpython_marshal(&source, "ordinary_for_cleanup.py");
    }
}
