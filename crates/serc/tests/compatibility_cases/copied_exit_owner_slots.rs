use crate::common::assert_matches_cpython_marshal;

#[test]
fn continue_copies_preserve_unwound_finally_slots() {
    for (filename, source) in [
        (
            "nested_outer_finally_two_continues.py",
            r"def run(groups, api):
    try:
        for group in groups:
            try:
                for item in group:
                    resource = None
                    try:
                        if api.skip(item):
                            api.record(item)
                            continue
                        resource = api.open(item)
                        if api.failed(resource):
                            api.record(resource)
                            continue
                    finally:
                        if resource:
                            api.close(resource)
                        api.finish_item()
            finally:
                api.finish_group()
    finally:
        api.finish_all()
",
        ),
        (
            "nested_outer_finally_one_continue.py",
            r"def run(groups, api):
    try:
        for group in groups:
            try:
                for item in group:
                    resource = None
                    try:
                        if api.skip(item):
                            api.record(item)
                            continue
                        resource = api.open(item)
                    finally:
                        if resource:
                            api.close(resource)
                        api.finish_item()
            finally:
                api.finish_group()
    finally:
        api.finish_all()
",
        ),
        (
            "nested_outer_finally_coroutine.py",
            r"async def run(groups, api):
    try:
        for group in groups:
            try:
                for item in group:
                    resource = None
                    try:
                        if api.skip(item):
                            api.record(item)
                            continue
                        resource = api.open(item)
                        if api.failed(resource):
                            api.record(resource)
                            continue
                    finally:
                        if resource:
                            api.close(resource)
                        api.finish_item()
            finally:
                api.finish_group()
    finally:
        api.finish_all()
",
        ),
        (
            "old_coroutine_single_finally.py",
            r"async def run(items, api):
    for item in items:
        try:
            if api.skip(item):
                api.record(item)
                continue
            api.use(item)
        finally:
            if item:
                api.close(item)
            api.finish_item()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn constant_handler_truth_tests_preserve_cleanup_owners() {
    for (filename, source) in [
        (
            "handler_truth_not_two_calls.py",
            r"async def run(flag):
    try:
        await work()
    except Exception:
        report0()
        report1()
        return None
    finally:
        if not flag:
            cleanup()
        finish()
",
        ),
        (
            "handler_truth_positive_two_calls.py",
            r"async def run(flag):
    try:
        await work()
    except Exception:
        report0()
        report1()
        return None
    finally:
        if flag:
            cleanup()
        finish()
",
        ),
        (
            "handler_truth_call_control.py",
            r"async def run(flag):
    try:
        await work()
    except Exception:
        report0()
        report1()
        return None
    finally:
        if check(flag):
            cleanup()
        finish()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn async_manager_returns_preserve_outer_try_slots() {
    for (filename, source) in [
        (
            "async_manager_two_outer_tries.py",
            r"async def run(manager, flag):
    try:
        try:
            async with manager:
                return await work()
        except ValueError:
            report()
            raise
    except RuntimeError:
        report_outer()
        raise
    finally:
        if flag is not None:
            await cleanup(flag)
",
        ),
        (
            "async_manager_inner_ordinary_try_control.py",
            r"async def run(manager, flag):
    try:
        async with manager:
            try:
                return await work()
            except ValueError:
                report()
                raise
    finally:
        if flag is not None:
            await cleanup(flag)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn retained_returns_preserve_three_pop_slots() {
    for (filename, source) in [
        (
            "retained_return_three_pops_local.py",
            r#"async def f(session, receiver, flag):
    try:
        try:
            if flag:
                await receiver.fail()
                status = "failed"
                return
        except InnerError:
            recover()
    except OuterError:
        raise
    finally:
        if session is not None:
            await session.close()
"#,
        ),
        (
            "retained_return_three_pops_cell.py",
            r#"async def f(session, receiver, flag):
    def keep_session():
        return session

    try:
        try:
            if flag:
                await receiver.fail()
                status = "failed"
                return
        except InnerError:
            recover()
    except OuterError:
        raise
    finally:
        if session is not None:
            await session.close()
"#,
        ),
        (
            "retained_return_two_pops_control.py",
            r#"async def f(session, receiver, flag):
    def keep_session():
        return session

    try:
        try:
            if flag:
                await receiver.fail()
                status = "failed"
                return
        except InnerError:
            recover()
    finally:
        if session is not None:
            await session.close()
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
