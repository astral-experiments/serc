use crate::common;

#[test]
fn copied_handler_return_none_owner_target() {
    let source = r"def run(value):
    try:
        perform()
    except Error:
        report('failed')
        return Result(loaded=False, version=None)
    finally:
        if value is not None and value.ready:
            value.close()
";
    common::assert_matches_cpython_marshal(source, "copied_handler_return_none_owner_target.py");
}

#[test]
fn copied_handler_return_none_owner_one_call_control() {
    let source = r"def run(value):
    try:
        perform()
    except Error:
        return Result(loaded=False, version=None)
    finally:
        if value is not None and value.ready:
            value.close()
";
    common::assert_matches_cpython_marshal(
        source,
        "copied_handler_return_none_owner_one_call_control.py",
    );
}

#[test]
fn named_handler_inner_iterator_return_none() {
    let source = r"def f(items, value):
    try:
        work()
    except Exception as error:
        for item in items:
            try:
                if item:
                    report(error)
                    return
            except LookupError:
                recover()
    finally:
        if value is not None:
            value.close()
";
    common::assert_matches_cpython_marshal(source, "named_handler_inner_iterator_return_none.py");
}
