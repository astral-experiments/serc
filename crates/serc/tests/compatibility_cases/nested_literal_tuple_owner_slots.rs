use crate::common;

#[test]
fn matches_cpython_nested_literal_tuple_slots_before_try() {
    for value in [
        "((name, ()),)",
        "((0, ()), (name, ()))",
        "((0, ()), (name, ()), (1, ()))",
        "((0, (1, (2, ()))), (name, ()), (3, ()))",
        "(\n    (0, ()),\n    (name, ()),\n    (1, ()),\n)",
        "((0, ()), (1, ()), (2, ()))",
    ] {
        let body = format!(
            "values = {value}
try:
    if flag:
        value = left()
    else:
        value = right()
    finish()
except:
    recover()
    raise
"
        );
        for scope in ["", "if active:\n", "def f(name, flag):\n", "class C:\n"] {
            let source = if scope.is_empty() {
                body.clone()
            } else {
                format!("{scope}{}", common::indent(&body))
            };
            common::assert_matches_cpython_marshal(&source, "nested_literal_tuple_slots.py");
        }
    }
}
