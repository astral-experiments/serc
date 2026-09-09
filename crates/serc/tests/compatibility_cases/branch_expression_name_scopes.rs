use crate::common;

#[test]
fn matches_cpython_marshal_for_branch_expression_name_scopes() {
    for expression in [
        "(transform(item) for item in source())",
        "(lambda argument=default(): transform(argument))",
        "consume(*(item.method() for item in values))",
        "[item.method() for item in values]",
        "tuple(item.method() for item in values)",
        "any(check(item) for item in values)",
    ] {
        for statement in [
            format!("    obj.result = {expression} if flag else fallback()\n"),
            format!("    return {expression} if True else fallback()\n"),
            format!("    if False:\n        obj.result = {expression}\n    return fallback()\n"),
        ] {
            let source = format!("def f(obj, values, flag):\n{statement}");
            common::assert_matches_cpython_marshal(&source, "branch_expression_name_scopes.py");
        }
    }
}
