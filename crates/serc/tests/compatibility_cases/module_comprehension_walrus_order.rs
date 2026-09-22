use crate::common;

#[test]
fn matches_cpython_marshal_for_module_comprehension_walrus_order() {
    for (name, source) in [
        (
            "key",
            "result = {(bound := outer): [bound for inner in values] for outer in values}\n",
        ),
        (
            "value",
            "result = {outer: ((bound := outer), [bound for inner in values]) for outer in values}\n",
        ),
        (
            "nested_target",
            "result = [((first := (second := outer)), [first for inner in values]) for outer in values]\n",
        ),
        (
            "dict_symbols",
            "result = [{(first := outer): (second := outer), (third := outer): [first for inner in values]} for outer in values]\n",
        ),
        (
            "outer_read",
            "result = [(bound, [(bound := inner) for inner in values]) for outer in values]\n",
        ),
        (
            "child_binding",
            "result = [[(bound := inner) for inner in values] for outer in values]\n",
        ),
        (
            "function",
            "def f():\n    return {(bound := outer): [bound for inner in values] for outer in values}\n",
        ),
        (
            "ordinary_global",
            "result = {outer: [bound for inner in values] for outer in values}\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, &format!("module_walrus_{name}.py"));
    }
}
