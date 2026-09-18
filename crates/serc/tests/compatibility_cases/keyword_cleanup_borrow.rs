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

#[test]
fn matches_cpython_marshal_for_original_jump_keyword_cleanup_witness() {
    for (filename, source) in [
        (
            "normal_finally_surviving_jump_keyword_witness_continued.py",
            r#"async def f(self, choice, stop, manager, work, report):
    try:
        if choice:
            if stop:
                return
            with manager:
                await work()
        else:
            with manager:
                await work()
    except Exception as error:
        report(error)
    finally:
        self.pending -= 1
        report(self.pending, tags=self.tags)
    report("done")
"#,
        ),
        (
            "normal_finally_surviving_jump_positional_control_continued.py",
            r#"async def f(self, choice, stop, manager, work, report):
    try:
        if choice:
            if stop:
                return
            with manager:
                await work()
        else:
            with manager:
                await work()
    except Exception as error:
        report(error)
    finally:
        self.pending -= 1
        report(self.pending, self.tags)
    report("done")
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_original_jump_primitive_cleanup_witness() {
    for (filename, source) in [
        (
            "normal_finally_nested_try_primitive_witness.py",
            r#"def f(flag, owned, source, root, work, cleanup):
    try:
        try:
            if flag:
                work(source, directory=root)
            else:
                work(root, directory=source)
        except OSError as error:
            raise RuntimeError("failed") from error
    except (RuntimeError, OSError):
        if owned:
            try:
                work(source)
            except OSError:
                pass
        raise
    finally:
        if owned:
            cleanup(source)
        cleanup(root)
"#,
        ),
        (
            "normal_finally_nested_try_call_witness_control.py",
            r#"def f(flag, owned, source, root, work, cleanup):
    try:
        try:
            if flag:
                work(source, directory=root)
            else:
                work(root, directory=source)
        except OSError as error:
            raise RuntimeError("failed") from error
    except (RuntimeError, OSError):
        if owned:
            try:
                work(source)
            except OSError:
                pass
        raise
    finally:
        cleanup(source)
        cleanup(root)
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_retained_conditional_finally_entry() {
    for (filename, source) in [
        (
            "normal_finally_async_return_arm_named_handler.py",
            r#"async def f(self, ready, manager, report):
    status = "success"
    count = 0
    try:
        if ready:
            async with manager as response:
                result = await response.read()
                count = len(result)
                return result
    except Exception as error:
        report(error)
        status = "error"
    finally:
        elapsed = self.clock() - self.started
        tags = (status, count, self.tags)
        report(elapsed, tags)
    return None
"#,
        ),
        (
            "normal_finally_async_return_arm_linear_tail_control.py",
            r#"async def f(self, ready, manager, report):
    status = "success"
    count = 0
    try:
        if ready:
            async with manager as response:
                result = await response.read()
                count = len(result)
                return result
        report(ready)
    except Exception as error:
        report(error)
        status = "error"
    finally:
        elapsed = self.clock() - self.started
        tags = (status, count, self.tags)
        report(elapsed, tags)
    return None
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
