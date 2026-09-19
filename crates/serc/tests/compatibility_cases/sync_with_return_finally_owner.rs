use crate::common;

#[test]
fn matches_cpython_marshal_for_sync_with_return_finally_owner_target() {
    let source = r"async def run(resource):
    try:
        with manager():
            return build(value=resource)
    finally:
        if resource is not None:
            await resource.close()
";
    common::assert_matches_cpython_marshal(source, "sync_with_return_finally_owner_target.py");
}

#[test]
fn matches_cpython_marshal_for_sync_with_return_finally_owner_control() {
    let source = r"async def run(resource):
    try:
        with manager():
            return build(value=resource)
    except Error:
        raise
    finally:
        if resource is not None:
            await resource.close()
";
    common::assert_matches_cpython_marshal(source, "sync_with_return_finally_owner_control.py");
}
