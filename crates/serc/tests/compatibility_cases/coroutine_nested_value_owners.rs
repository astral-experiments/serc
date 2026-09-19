use crate::common;

#[test]
fn coroutine_nested_value_or() {
    let source = r"async def parse_context(session):
    value = Model.validate((session.primary or session.secondary) or {})
    return value
";
    common::assert_matches_cpython_marshal(source, "coroutine_nested_value_or.py");
}

#[test]
fn coroutine_flat_value_or_control() {
    let source = r"async def parse_context(session):
    value = Model.validate(session.primary or session.secondary or {})
    return value
";
    common::assert_matches_cpython_marshal(source, "coroutine_flat_value_or_control.py");
}
