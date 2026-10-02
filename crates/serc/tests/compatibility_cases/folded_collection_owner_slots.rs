use crate::common;

#[test]
fn matches_cpython_guard_owners_after_starred_literal_prefixes() {
    for (open, close) in [("[", "]"), ("(", ")"), ("{", "}")] {
        for prefix in [
            "1, 2",
            "1, 2, 3",
            "1, 2, 3, 4, 5, 6",
            "(1, 2), (3, 4), (5, 6)",
        ] {
            let source = format!(
                "def f(values, flag):
    with manager() as state:
        result = prepare(state)
        active = False
        try:
            initialize(state)
            active = True
            selected = [item for item in collect({open}{prefix}, *values{close}, root=result).splitlines() if item != '']
            if len(selected) == 0:
                raise Error('empty')
            use({open}{prefix}, *selected{close}, root=result)
            finish(['a', 'b', 'c', flag.attr], root=result)
        finally:
            if active:
                close(state)
    done()
"
            );
            common::assert_matches_cpython_marshal(&source, "starred_prefix_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_finally_owners_after_literal_collection_folding() {
    for (definition, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    if False:\n        yield v0\n"),
        ("async def", "    if False:\n        yield v0\n"),
    ] {
        for collection in [
            "[0.1, 0.5, 0.9]",
            "[0.1, 0.5, 0.9, 1.1]",
            "[0.1, 0.5, 0.9, 1.1, 1.5]",
            "{0.1, 0.5, 0.9}",
            "{0.1, 0.5, 0.9, 1.1}",
            "{0.1, 0.5, 0.9, 1.1, 1.5}",
        ] {
            let source = format!(
                "{definition} f(v0, v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14):
{suspension}    try:
        if v0 is None:
            pass
        first, second, third = v1.method(\"x\", {collection}, float(v2))
        print(f\"result={{float(third):.6f}}\")
    finally:
        if v1 is not None:
            pass
"
            );
            common::assert_matches_cpython_marshal(&source, "literal_collection_owner_slots.py");
        }
    }
}

#[test]
fn preserves_finally_owners_for_other_collection_folds() {
    for collection in [
        "[0.1, 0.5, 0.9, 1.1, 1.5, 1.9]",
        "{0.1, 0.5, 0.9, 1.1, 1.5, 1.9}",
        "(0.1, 0.5, 0.9)",
        "[1 + 2, 3, 4]",
        "[-1, 2, 3]",
        "[v0, 2, 3]",
    ] {
        let source = format!(
            "def f(v0, v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13):
    try:
        if v0 is None:
            pass
        first, second, third = v1.method(\"x\", {collection}, float(v2))
        print(f\"result={{float(third):.6f}}\")
    finally:
        if v1 is not None:
            pass
"
        );
        common::assert_matches_cpython_marshal(&source, "other_collection_owner_slots.py");
    }
}
