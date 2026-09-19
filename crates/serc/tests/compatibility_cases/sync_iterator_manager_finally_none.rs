use crate::common;

#[test]
fn sync_iterator_manager_finally_none_target() {
    let source = r"def run(manager, response, chunks, selected):
    try:
        with manager:
            for chunk in chunks:
                if selected:
                    return make_result(convert(response))
    except Error:
        raise
    finally:
        if response is not None:
            response.close()
";
    common::assert_matches_cpython_marshal(source, "sync_iterator_manager_finally_none_target.py");
}

#[test]
fn sync_iterator_manager_finally_none_one_call_control() {
    let source = r"def run(manager, response, chunks, selected):
    try:
        with manager:
            for chunk in chunks:
                if selected:
                    return make_result(response)
    except Error:
        raise
    finally:
        if response is not None:
            response.close()
";
    common::assert_matches_cpython_marshal(
        source,
        "sync_iterator_manager_finally_none_one_call_control.py",
    );
}

#[test]
fn matches_cpython_coroutine_inner_try_manager_none_prefixes() {
    for (filename, source) in [
        (
            "coroutine_inner_try_iterator_manager_none_finally.py",
            r"async def target(manager, items, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                for item in items:
                    if item:
                        session.rollback()
                        return False
            except Exception:
                raise
            return True
    finally:
        if client is not None:
            await client.close()
",
        ),
        (
            "coroutine_inner_try_manager_none_finally.py",
            r"async def target(manager, condition, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                if condition:
                    session.rollback()
                    return False
            except Exception:
                raise
            return True
    finally:
        if client is not None:
            await client.close()
",
        ),
        (
            "coroutine_manager_none_finally_no_inner_try_control.py",
            r"async def target(manager, client):
    try:
        with manager() as session:
            def current_session():
                return session
            session.rollback()
            return False
    finally:
        if client is not None:
            await client.close()
",
        ),
        (
            "coroutine_unnamed_handler_manager_none_finally.py",
            r"async def target(manager, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                await session.work()
            except ValueError:
                session.rollback()
                return False
            return True
    finally:
        if client is not None:
            await client.close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
