use crate::common;

#[test]
fn nested_manager_none_exact_null_target() {
    let source = r"def run(context, flag):
    with (
        context.manager(1),
        context.manager(2),
        context.manager(3),
        context.manager(4) as stream,
    ):
        value = stream.read() if flag is None else stream
        consume(value)
";
    common::assert_matches_cpython_marshal(source, "nested_manager_none_exact_null_target.py");
}

#[test]
fn nested_manager_none_fewer_managers_owned_control() {
    let source = r"def run(context, flag):
    with (
        context.manager(1),
        context.manager(2),
        context.manager(3) as stream,
    ):
        value = stream.read() if flag is None else stream
        consume(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_manager_none_fewer_managers_owned_control.py",
    );
}

#[test]
fn nested_manager_none_earlier_manager_owned_control() {
    let source = r"def run(context, flag):
    with (
        context.manager(1),
        context.manager(2),
        context.manager(3),
        context.manager(4),
        context.manager(5) as stream,
    ):
        value = stream.read() if flag is None else stream
        consume(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_manager_none_earlier_manager_owned_control.py",
    );
}
