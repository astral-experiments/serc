#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_terminal_with_and_raises() {
    let source = r#"def two_operands(manager, dtype_is_int, value):
    with manager:
        if not dtype_is_int and not valid(value):
            raise ValueError("invalid")

def nested_boolean(manager, accept, instance, value):
    with manager:
        if (
            not accept
            and not instance.ignore
            and (is_sparse(value) and any_negative(value))
        ):
            raise ValueError("negative")

def three_operands(manager, check, value, method):
    with manager:
        if check and method == "box-cox" and minimum(value) <= 0:
            raise ValueError("not positive")
"#;
    common::assert_matches_cpython_marshal(source, "with_terminal_and_raises.py");
}
