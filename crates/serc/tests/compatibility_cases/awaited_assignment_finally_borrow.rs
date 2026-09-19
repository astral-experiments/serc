use crate::common;

#[test]
fn matches_cpython_marshal_for_awaited_assignment_finally_borrow_target() {
    let source = r"async def load(client, manager, telemetry, allow_failure=False):
    try:
        async with manager as context:
            result = None
            started = clock()
            try:
                result = await client.complete(mode=allow_failure)
            except CompletionError as error:
                if not allow_failure:
                    raise error
                record_expected()
            finally:
                telemetry.elapsed = seconds_to_millis(clock() - started)
            if allow_failure:
                context.outcome = 'completed'
                return None
            assert result is not None
            record_complete()
            return result
    except Exception as error:
        record_error(error)
        return None
";
    common::assert_matches_cpython_marshal(source, "awaited_assignment_finally_borrow_target.py");
}

#[test]
fn matches_cpython_marshal_for_awaited_assignment_finally_borrow_discard_control() {
    let source = r"async def load(client, manager, telemetry, allow_failure=False):
    try:
        async with manager as context:
            result = None
            started = clock()
            try:
                await client.complete(mode=allow_failure)
            except CompletionError as error:
                if not allow_failure:
                    raise error
                record_expected()
            finally:
                telemetry.elapsed = seconds_to_millis(clock() - started)
            if allow_failure:
                context.outcome = 'completed'
                return None
            assert result is not None
            record_complete()
            return result
    except Exception as error:
        record_error(error)
        return None
";
    common::assert_matches_cpython_marshal(source, "awaited_discard_finally_borrow_control.py");
}
