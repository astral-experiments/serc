use crate::common::assert_matches_cpython_marshal;

#[test]
fn calculated_named_handler_return_three_iterators() {
    assert_matches_cpython_marshal(
        r"async def render(self, request, payload, groups, session, network_enabled):
    try:
        for group in groups:
            for path in group:
                for attempt in range(3):
                    try:
                        success, output = await run_command(session, path)
                    except Exception as error:
                        from transport import CommandError
                        if isinstance(error, CommandError):
                            logger.error(
                                'Command failed',
                                status=error.status,
                                output=error.output.decode('utf-8', errors='replace'),
                                request_id=request.identifier,
                                attempt=attempt + 1,
                            )
                        return self.failure_output(
                            request_id=request.identifier,
                            payload=payload,
                            error_label='command failed',
                            metadata={
                                'error': 'command failed',
                                'exception': repr(error),
                                'attempt': attempt + 1,
                            },
                        )
                    if success:
                        break
        return self.success_output(request.identifier)
    finally:
        if network_enabled:
            try:
                await session.update_network(False)
                logger.info('Network disabled', request_id=request.identifier)
            except Exception:
                logger.exception('Network cleanup failed', request_id=request.identifier)
        if session is not None:
            try:
                await session.close()
            except Exception:
                logger.exception('Session cleanup failed', request_id=request.identifier)
",
        "calculated_named_handler_return_three_iterators.py",
    );
}

#[test]
fn matches_cpython_protected_iterator_while_value_return_depths() {
    for (filename, source) in [
        (
            "protected_sync_iterator_while_value_returns.py",
            r#"def one_iterator(values, done):
    try:
        for value in values:
            while ready(value):
                if done(value):
                    return {"items": value}
                advance(value)
    except Exception:
        recover()
    return None


def two_iterators(rows, done):
    try:
        for row in rows:
            while ready(row):
                for value in row:
                    while ready(value):
                        if done(value):
                            return {"items": value}
                        advance(value)
                advance(row)
    except Exception:
        recover()
    return None
"#,
        ),
        (
            "protected_sync_iterator_while_depth_controls.py",
            r#"def direct_iterator(values, done):
    try:
        for value in values:
            if done(value):
                return {"items": value}
    except Exception:
        recover()
    return None


def literal_under_while(values, done):
    try:
        for value in values:
            while ready(value):
                if done(value):
                    return None
                advance(value)
    except Exception:
        recover()
    return None


def newer_try_inside_iterator(values, done):
    try:
        for value in values:
            try:
                while ready(value):
                    if done(value):
                        return {"items": value}
                    advance(value)
            except ValueError:
                recover_inner()
    except Exception:
        recover()
    return None
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
