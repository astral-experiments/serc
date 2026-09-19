use crate::common;

#[test]
fn nested_scope_constants_precede_later_while_body_literals() {
    for scope in ["def f():\n", "async def f():\n", "def f():\n    yield\n"] {
        for condition in ["ready()", "not ready()"] {
            for nested in [
                "def value():\n    pass",
                "class value:\n    pass",
                "value = lambda: None",
                "value = call(arg=lambda: None)",
            ] {
                for call in ["consume(arg=value)", "consume(*args, arg=value)"] {
                    let body = format!("{nested}\n{call}\nif stop():\n    break");
                    let source = format!(
                        "{scope}    while {condition}:\n        {}\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_scope_first_constant_order.py",
                    );
                }
            }
        }
    }
}
