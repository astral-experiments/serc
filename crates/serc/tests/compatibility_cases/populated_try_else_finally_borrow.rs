use crate::common;

#[test]
fn populated_try_else_finally_borrow_target() {
    let source = r"async def run(open_stream, manager, progress, observe):
    result = ''
    failure = None
    try:
        stream = await open_stream()
        async with manager:
            value = await stream.read()
    except Exception as error:
        failure = str(error)
    else:
        result = extract(value.output)
    finally:
        if progress:
            status = 'ok' if failure is None else 'error'
            await progress.bump(status)
    if failure:
        return '', failure
    return result, None
";
    common::assert_matches_cpython_marshal(source, "populated_try_else_finally_borrow_target.py");
}

#[test]
fn populated_try_else_finally_borrow_body_tail_control() {
    let source = r"async def run(open_stream, manager, progress, observe):
    result = ''
    failure = None
    try:
        stream = await open_stream()
        async with manager:
            value = await stream.read()
        observe()
    except Exception as error:
        failure = str(error)
    else:
        result = extract(value.output)
    finally:
        if progress:
            status = 'ok' if failure is None else 'error'
            await progress.bump(status)
    if failure:
        return '', failure
    return result, None
";
    common::assert_matches_cpython_marshal(
        source,
        "populated_try_else_finally_borrow_body_tail_control.py",
    );
}
