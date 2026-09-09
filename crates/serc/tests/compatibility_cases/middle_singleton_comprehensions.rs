use crate::common;

#[test]
fn matches_cpython_singleton_bindings_before_nested_comprehension_loops() {
    for (opening, closing, element) in [
        ("[", "]", "combine(item0, item1, item2)"),
        ("{", "}", "combine(item0, item1, item2)"),
        ("{", "}", "item0: combine(item0, item1, item2)"),
    ] {
        for clauses in [
            "for item0 in values for item1 in [choose(item0)] for item2 in values",
            "for item0 in values for item1 in (choose(item0),) if flag for item2 in values",
            "for item0 in values for item1 in [choose(item0)] for item2 in [choose(item1)]",
            "for item0 in values for item1 in [choose(item0)] if item1 and flag for item2 in values for item3 in (choose(item2),)",
        ] {
            for prefix in ["", "async "] {
                let source = format!(
                    "{prefix}def f(values, flag):\n    return {opening}{element} {clauses}{closing}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "middle_singleton_comprehension.py",
                );
            }
        }
    }
}
