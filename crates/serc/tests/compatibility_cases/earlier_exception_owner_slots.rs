use crate::common;

#[test]
fn matches_cpython_finally_owner_after_negative_literal_folding() {
    for literal in ["-1", "-257", "-1.5", "-2j"] {
        for calls in [1, 2, 3, 6] {
            for condition in ["a", "a is not None", "a >= 0"] {
                let body = "        use(a, b, c, d)\n".repeat(calls);
                let source = format!(
                    "def f(a, b, c, d):\n    try:\n{body}        value = {literal}\n    finally:\n        if {condition}:\n            cleanup()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "negative_literal_finally_owner.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_negative_literal_owner_across_scope_and_branch_boundaries() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for branch in ["", "        if flag:\n            value = -257\n"] {
            let source = format!(
                "{definition} f(a, b, c, d, flag):\n    try:\n        use(a, b, c, d)\n        use(a, b, c, d)\n{branch}        value = -1\n    finally:\n        if a is not None:\n            cleanup()\n{tail}"
            );
            common::assert_matches_cpython_marshal(&source, "negative_literal_owner_boundary.py");
        }
    }
}

#[test]
fn matches_cpython_normal_finally_reuses_earlier_try_owner() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for calls in [1, 2, 3, 4, 8] {
            for condition in ["value is not None", "value"] {
                let body = "        work()\n".repeat(calls);
                let source = format!(
                    "{definition} f(value, obj, flag, limit, manager):\n    try:\n{body}    finally:\n        if {condition}:\n            after()\n{tail}"
                );
                common::assert_matches_cpython_marshal(&source, "earlier_try_owner_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_finally_owner_after_source_branch_join() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        let source = format!(
            "{definition} f(value, obj, flag, limit, manager):\n    try:\n        if flag:\n            before()\n        work()\n        work()\n        work()\n    finally:\n        if value is not None:\n            after()\n{tail}"
        );
        common::assert_matches_cpython_marshal(&source, "source_join_finally_owner.py");
    }
}

#[test]
fn matches_cpython_finally_owner_slot_before_protected_prefix() {
    for condition in ["value is not None", "value"] {
        let prefix = "    before()\n".repeat(100);
        let source = format!(
            "def f(value):\n{prefix}    try:\n        work()\n        work()\n        work()\n    finally:\n        if {condition}:\n            after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "before_protected_prefix_owner.py");
    }
}

#[test]
fn matches_cpython_targeted_finally_entry_rejects_prior_array_history() {
    for definition in ["def", "async def"] {
        for body in [
            "        if flag:\n            work()\n            work()\n            work()\n",
            "        if flag:\n            work()\n            work()\n            work()\n        else:\n            other()\n            other()\n            other()\n",
        ] {
            for condition in ["value is not None", "value"] {
                let source = format!(
                    "{definition} f(value, flag):\n    try:\n{body}    finally:\n        if {condition}:\n            after()\n"
                );
                common::assert_matches_cpython_marshal(&source, "targeted_finally_entry_owner.py");
            }
        }
    }
}
