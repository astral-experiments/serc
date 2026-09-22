use crate::common;

#[test]
fn matches_cpython_pruned_class_qualified_name_origins() {
    for (prefix, canonical) in [
        ("", true),
        ("value = 'outer.<locals>.C'\n", false),
        ("if False:\n    value = 'outer.<locals>.C'\n", false),
        (
            "def discarded():\n    return None\n    value = 'outer.<locals>.C'\n",
            false,
        ),
        (
            "if False:\n    def outer():\n        class C: pass\n        return C\n",
            true,
        ),
    ] {
        let source = format!(
            "{prefix}def outer():\n    class C: pass\n    return C\n\
             code = next(value for value in outer.__code__.co_consts if getattr(value, 'co_name', None) == 'C')\n\
             print(code.co_consts[0] is code.co_qualname)\n"
        );
        common::assert_matches_cpython_marshal(&source, "pruned_qualified_name_origins.py");
        common::assert_executes(&source, if canonical { "True\n" } else { "False\n" });
    }
}
