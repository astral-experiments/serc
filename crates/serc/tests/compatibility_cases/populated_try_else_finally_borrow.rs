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

#[test]
fn try_else_conditional_call_finally_loop_nop_target() {
    let source = r"def f(futures, errors, outputs, progress):
    for future in futures:
        try:
            value = future.result()
        except Exception as exc:
            errors.append(exc)
        else:
            if value is None:
                errors.append(value)
            elif value:
                outputs.append(value)
        finally:
            progress.update(1)
";
    common::assert_matches_cpython_marshal(
        source,
        "try_else_conditional_call_finally_loop_nop_target.py",
    );
}

#[test]
fn try_else_nested_call_finally_closure_nop_target() {
    let source = r"def outer(futures, errors, outputs, metrics, progress):
    def drain():
        for future in futures:
            try:
                value, metric = future.result()
            except Exception as exc:
                errors.append(exc)
            else:
                if value is None:
                    errors.append(value)
                elif value:
                    outputs.append(value)
                    if metric is not None:
                        metrics.append(metric)
            finally:
                progress.update(1)
    return drain
";
    common::assert_matches_cpython_marshal(
        source,
        "try_else_nested_call_finally_closure_nop_target.py",
    );
}

#[test]
fn try_else_conditional_call_outer_finally_async_nop_target() {
    let source = r"async def f(futures, errors, outputs, progress):
    try:
        for future in futures:
            try:
                value = await future
            except Exception as exc:
                errors.append(exc)
            else:
                if value is None:
                    errors.append(value)
                else:
                    outputs.append(value)
                    if value.ready():
                        flush()
            finally:
                progress.update(1)
    finally:
        progress.close()
";
    common::assert_matches_cpython_marshal(
        source,
        "try_else_conditional_call_outer_finally_async_nop_target.py",
    );
}

#[test]
fn try_else_linear_call_finally_loop_nop_control() {
    let source = r"def f(futures, errors, outputs, progress):
    for future in futures:
        try:
            value = future.result()
        except Exception as exc:
            errors.append(exc)
        else:
            outputs.append(value)
        finally:
            progress.update(1)
";
    common::assert_matches_cpython_marshal(
        source,
        "try_else_linear_call_finally_loop_nop_control.py",
    );
}
