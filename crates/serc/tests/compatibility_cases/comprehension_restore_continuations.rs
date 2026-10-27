use crate::common;

const PARAMETERS: &str = "pad0, pad1, pad2, pad3, pad4, pad5, pad6, pad7, pad8, pad9, pad10, pad11, pad12, pad13, pad14, pad15, flag, manager, value, values, gate";

fn assert_restore_owner(statement: &str, prelude_calls: usize, join: &str, condition: &str) {
    let prelude = "    prepare()\n".repeat(prelude_calls);
    let source = format!(
        "def f({PARAMETERS}):\n    {statement}\n{prelude}{join}    try:\n        result = manager.work(\n            k0=value,\n            k1=value,\n            k2=value,\n            k3=value,\n        )\n    finally:\n        if {condition}:\n            cleanup()\n"
    );
    common::assert_matches_cpython_marshal(&source, "comprehension_restore_continuations.py");
}

#[test]
fn matches_cpython_marshal_for_comprehension_restore_continuations() {
    // One saved local needs no inter-restore boundary; two and three exercise it.
    for [single, pair, triple] in [
        [
            "[v0 for v0 in values]",
            "[v1 for v0, v1 in values]",
            "[v2 for v0, v1, v2 in values]",
        ],
        [
            "{v0 for v0 in values}",
            "{v1 for v0, v1 in values}",
            "{v2 for v0, v1, v2 in values}",
        ],
        [
            "{v0: v0 for v0 in values}",
            "{v0: v1 for v0, v1 in values}",
            "{v0: v2 for v0, v1, v2 in values}",
        ],
    ] {
        for (expression, condition, prelude_calls) in [
            (single, "flag is not None", 2),
            (pair, "flag is not None", 2),
            (pair, "flag", 4),
            (triple, "flag is not None", 2),
            (triple, "flag", 4),
        ] {
            assert_restore_owner(
                &format!("extracted = {expression}"),
                prelude_calls,
                "",
                condition,
            );
        }
    }
}

#[test]
fn preserves_real_boundaries_after_comprehension_restoration() {
    // Nested iterable restores retain their own entry; real joins and discarded
    // results must keep their existing exception-owner history.
    for (statement, prelude_calls, join, condition) in [
        (
            "extracted = [v1 for v0, v1 in [item for item in values]]",
            0,
            "",
            "flag is not None",
        ),
        (
            "extracted = [v2 for v0, v1, v2 in [item for item in values]]",
            2,
            "",
            "flag is not None",
        ),
        (
            "extracted = {v0: v1 for v0, v1 in [item for item in values]}",
            0,
            "",
            "flag is not None",
        ),
        (
            "extracted = {v0: v2 for v0, v1, v2 in [item for item in values]}",
            2,
            "",
            "flag is not None",
        ),
        (
            "extracted = [v1 for v0, v1 in values]",
            2,
            "    if gate:\n        prepare()\n    else:\n        settle()\n",
            "flag is not None",
        ),
        (
            "extracted = {v0: v2 for v0, v1, v2 in values}",
            2,
            "    for entry in values:\n        observe(entry)\n",
            "flag is not None",
        ),
        ("[v0 for v0 in values]", 4, "", "flag"),
        ("{v0 for v0 in values}", 4, "", "flag"),
        ("{v0: v0 for v0 in values}", 4, "", "flag"),
    ] {
        assert_restore_owner(statement, prelude_calls, join, condition);
    }
}

#[test]
fn fresh_try_after_comprehension_leaves_monitoring_marker_unprotected() {
    let source = "async def collect(owner, values):\n    owner.result = [format(value) for value in values]\n    try:\n        if active:\n            pass\n    except ValueError as error:\n        pass\n";
    common::assert_matches_cpython_marshal(source, "comprehension_then_try.py");
}
