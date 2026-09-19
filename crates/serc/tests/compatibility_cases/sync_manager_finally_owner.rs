use crate::common;

#[test]
fn sync_two_manager_finally_none_owner_target() {
    let source = r"def run(first, second, resource, selected):
    try:
        with first, second:
            if selected:
                return make_result(resource)
            consume(resource)
    except Error:
        raise
    finally:
        if resource is not None:
            resource.close()
";
    common::assert_matches_cpython_marshal(source, "sync_two_manager_finally_none_owner_target.py");
}

#[test]
fn sync_two_manager_finally_none_null_control() {
    let source = r"def run(first, second, resource, selected):
    try:
        with first, second:
            if selected:
                return resource
            consume(resource)
    except Error:
        raise
    finally:
        if resource is not None:
            resource.close()
";
    common::assert_matches_cpython_marshal(source, "sync_two_manager_finally_none_null_control.py");
}
