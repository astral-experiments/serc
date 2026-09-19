use crate::common;

#[test]
fn matches_cpython_marshal_for_intervening_try_finally_copy() {
    let source = r#"async def fetch(session):
    try:
        response = await session.request()
        try:
            response.check()
            return await response.read()
        finally:
            response.release()
    except Exception as error:
        detail = str(error).strip()
        label = detail or type(error).__name__
        raise FetchError(f"Request failed: {label}") from error
    finally:
        await session.close()
"#;
    common::assert_matches_cpython_marshal(source, "intervening_try_finally_copy.py");
}

#[test]
fn matches_cpython_marshal_for_adjacent_finally_copy_control() {
    let source = r"async def fetch(session):
    try:
        response = await session.request()
        try:
            response.check()
            return await response.read()
        finally:
            response.release()
    finally:
        await session.close()
";
    common::assert_matches_cpython_marshal(source, "adjacent_finally_copy_control.py");
}
