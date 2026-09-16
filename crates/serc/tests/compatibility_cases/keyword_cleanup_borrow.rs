use crate::common;

#[test]
fn matches_cpython_marshal_for_keyword_cleanup_borrow_target() {
    let source = r"async def prepare_session(session, soft_fail=False):
    try:
        await install(session, ['base'], env={'SETUP': ''})
        await install(session, ['project'], catch=False)
        await install(session, ['checks'])
    except Exception:
        logger.exception('Setup failed', exc_info=True)
        if not soft_fail:
            raise
    finally:
        await session.container.update_network(enable_network=False)
        has_network = await container_has_network_access(session)
        if has_network:
            raise RuntimeError('Network access remains enabled')
";
    common::assert_matches_cpython_marshal(source, "keyword_cleanup_borrow_target.py");
}

#[test]
fn matches_cpython_marshal_for_keyword_cleanup_borrow_positional_control() {
    let source = r"async def prepare_session(session, soft_fail=False):
    try:
        await install(session, ['base'], env={'SETUP': ''})
        await install(session, ['project'], catch=False)
        await install(session, ['checks'])
    except Exception:
        logger.exception('Setup failed', exc_info=True)
        if not soft_fail:
            raise
    finally:
        await session.container.update_network(False)
        has_network = await container_has_network_access(session)
        if has_network:
            raise RuntimeError('Network access remains enabled')
";
    common::assert_matches_cpython_marshal(source, "positional_cleanup_borrow_control.py");
}

#[test]
fn matches_cpython_marshal_for_keyword_cleanup_borrow_fallthrough_handler_control() {
    let source = r"async def prepare_session(session, soft_fail=False):
    try:
        await install(session, ['base'], env={'SETUP': ''})
        await install(session, ['project'], catch=False)
        await install(session, ['checks'])
    except Exception:
        logger.exception('Setup failed', exc_info=True)
        if not soft_fail:
            report_recovery()
    finally:
        await session.container.update_network(enable_network=False)
        has_network = await container_has_network_access(session)
        if has_network:
            raise RuntimeError('Network access remains enabled')
";
    common::assert_matches_cpython_marshal(source, "fallthrough_handler_cleanup_borrow_control.py");
}
