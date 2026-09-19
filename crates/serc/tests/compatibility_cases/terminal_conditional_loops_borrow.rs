use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_conditional_loops_borrow_target() {
    let source = r"async def consume(select, batch, stream, decoder, emit, recover, mark, monitor, task, result):
    count = 0
    info = None
    try:
        if select:
            values = await batch()
            if values is None:
                raise ValueError()
            for value in values:
                emit(value)
                if value is not None:
                    count += 1
                if monitor and count:
                    monitor.update(count)
        else:
            async for chunk in stream:
                for value in chunk:
                    emit(value)
                    if value is not None:
                        count += 1
                    if monitor and count:
                        monitor.update(count)
    except Exception as error:
        info = f'{type(error).__name__}: {error}'
        if recover(info):
            mark()
    finally:
        try:
            trailing = decoder.decode(b'', final=True)
        except UnicodeDecodeError:
            trailing = ''
        emit(trailing, None)
        if task is not None:
            await task
    return result(count, info, task, monitor, select)
";
    common::assert_matches_cpython_marshal(source, "terminal_conditional_loops_borrow_target.py");
}

#[test]
fn matches_cpython_marshal_for_terminal_conditional_loops_borrow_live_tail_control() {
    let source = r"async def consume(select, batch, stream, decoder, emit, recover, mark, monitor, task, result):
    count = 0
    info = None
    try:
        if select:
            values = await batch()
            if values is None:
                raise ValueError()
            for value in values:
                emit(value)
                if value is not None:
                    count += 1
                if monitor and count:
                    monitor.update(count)
        else:
            async for chunk in stream:
                for value in chunk:
                    emit(value)
                    if value is not None:
                        count += 1
                    if monitor and count:
                        monitor.update(count)
        mark()
    except Exception as error:
        info = f'{type(error).__name__}: {error}'
        if recover(info):
            mark()
    finally:
        try:
            trailing = decoder.decode(b'', final=True)
        except UnicodeDecodeError:
            trailing = ''
        emit(trailing, None)
        if task is not None:
            await task
    return result(count, info, task, monitor, select)
";
    common::assert_matches_cpython_marshal(
        source,
        "terminal_conditional_loops_borrow_live_tail_control.py",
    );
}
