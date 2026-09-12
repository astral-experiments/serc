use crate::common;

#[test]
fn preserves_manager_pop_owner_after_conditional_raise() {
    let source = r"def f(manager, value):
    with manager:
        try:
            consume(value)
            if value is not None:
                raise Error(value)
        except Error:
            recover()
            raise
    after()
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_raise.py");
}

#[test]
fn preserves_manager_pop_owner_after_conditional_return() {
    let source = r"async def f(manager, value):
    with manager:
        try:
            value = await read(value)
            if value is not None:
                report(value)
                return value
        except Error:
            recover()
            raise
    after()
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_return.py");
}

#[test]
fn preserves_manager_pop_owner_after_explicit_else() {
    let source = r"def f(manager, values):
    for value in values:
        with manager:
            try:
                if first(value) and second(value) and value:
                    report(value)
                    return value
                else:
                    report(value)
            except Error:
                recover()
                continue
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_explicit_else.py");
}
