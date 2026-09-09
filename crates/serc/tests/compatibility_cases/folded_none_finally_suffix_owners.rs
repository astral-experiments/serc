use crate::common::assert_matches_cpython_marshal;
use std::fmt::Write as _;

#[test]
fn matches_cpython_folded_none_finally_suffix_owners() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for operator in ["is", "is not"] {
            for (left, calls) in [
                ("value", 1),
                ("value.attr", 2),
                ("items[0]", 3),
                ("task(value)", 4),
            ] {
                let asynchronous = matches!(kind, "coroutine" | "async_generator");
                let mut source = format!(
                    "{}def f(value, task, flag, manager, items, other):\n",
                    if asynchronous { "async " } else { "" },
                );
                if matches!(kind, "generator" | "async_generator") {
                    source.push_str("    yield value\n");
                }
                source.push_str("    with manager:\n        try:\n            if flag:\n");
                writeln!(
                    source,
                    "                {}",
                    if kind == "async_generator" {
                        "return"
                    } else {
                        "return value"
                    },
                )
                .unwrap();
                for _ in 0..calls {
                    source.push_str("            work()\n");
                }
                writeln!(
                    source,
                    "        finally:\n            if {left} {operator} None:\n                cleanup()",
                )
                .unwrap();
                assert_matches_cpython_marshal(
                    &source,
                    &format!("folded_none_finally_{kind}_{operator}_{calls}.py"),
                );
            }
        }
    }
}
