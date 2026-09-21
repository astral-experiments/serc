use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_marshal_for_normal_finally_after_coroutine_with_return() {
    for manager in ["with", "async with"] {
        for cleanup in ["cleanup(flag=True)", "await cleanup(flag=True)"] {
            for value in ["None", "value.read()"] {
                let source = format!(
                    "async def f(manager, cleanup):\n    try:\n        {manager} manager as value:\n            return {value}\n    finally:\n        {cleanup}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_with_exit_owner.py",
                );
            }
        }
    }
}

#[test]
fn preserves_manager_owners_on_complete_finally_branch_joins() {
    for body in ["work()", "yield None"] {
        for otherwise in ["finish()", "mapping[key] = value"] {
            for manager in [false, true] {
                let cleanup = format!(
                    "try:\n    {body}\nfinally:\n    if flag:\n        cleanup()\n    else:\n        {otherwise}\n"
                );
                let suite = if manager {
                    format!("with manager:\n{}", indent(&cleanup))
                } else {
                    cleanup
                };
                let source = format!(
                    "def f(manager, flag, mapping, key, value):\n{}",
                    indent(&suite)
                );
                common::assert_matches_cpython_marshal(&source, "finally_branch_join_owners.py");
            }
        }
    }
}

#[test]
fn preserves_finally_exits_with_one_reachable_arm() {
    for body in ["work()", "yield None"] {
        for abrupt in ["return", "raise Error"] {
            for (test, first, otherwise) in [
                ("flag", abrupt, "finish()"),
                ("False", abrupt, "finish()"),
                ("True", "cleanup()", abrupt),
            ] {
                let source = format!(
                    "def f(manager, flag):\n    with manager:\n        try:\n            {body}\n        finally:\n            if {test}:\n                {first}\n            else:\n                {otherwise}\n"
                );
                common::assert_matches_cpython_marshal(&source, "finally_single_arm_exit.py");
            }
        }
    }
}

#[test]
fn preserves_outer_manager_owner_on_conditional_finally_manager_joins() {
    for body in ["work()", "yield value"] {
        for leading in [
            "",
            "            cleanup()\n            flush()\n            seek(0)\n",
        ] {
            for condition in ["flag", "flag is not None"] {
                let source = format!(
                    "def f(outer, inner, flag, value):\n    with outer():\n        try:\n            {body}\n        finally:\n{leading}            if {condition}:\n                with inner() as resource:\n                    result = resource.read()\n                    work(result)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_finally_manager_join.py",
                );
            }
        }
    }
}

#[test]
fn preserves_outer_manager_owner_on_conditional_finally_handler_joins() {
    for (filename, source) in [
        (
            "manager_conditional_finally_recovery.py",
            r"def target(manager, work, cleanup, enabled):
    with manager:
        try:
            work()
        finally:
            if enabled:
                try:
                    cleanup()
                except ValueError:
                    pass
",
        ),
        (
            "manager_linear_finally_control.py",
            r"def target(manager, work, cleanup, enabled):
    with manager:
        try:
            work()
        finally:
            cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_outer_manager_owner_after_sequential_finally_guards() {
    for leading in [
        "            cleanup()\n",
        "            if ready:\n                cleanup()\n",
        "            if ready:\n                cleanup()\n                flush()\n",
    ] {
        let source = format!(
            "def f(outer, inner, ready, resource):\n    with outer():\n        try:\n            work()\n        finally:\n{leading}            if resource is not None:\n                with inner():\n                    resource.flush()\n"
        );
        common::assert_matches_cpython_marshal(&source, "sequential_finally_manager_joins.py");
    }
}

#[test]
fn preserves_normal_finally_attribute_owners_inside_async_managers() {
    for (filename, source) in [
        (
            "async_manager_finally_method_after_async_for.py",
            r"async def f(manager, task, consume):
    async with manager as stream:
        try:
            async for item in stream:
                if not item:
                    break
                await consume(item)
        finally:
            if not task.done():
                task.cancel()
                await task
",
        ),
        (
            "async_manager_finally_method_linear_control.py",
            r"async def f(manager, task, consume):
    async with manager as stream:
        try:
            await consume(stream)
        finally:
            if not task.done():
                task.cancel()
                await task
",
        ),
        (
            "async_manager_finally_attribute_after_conditional_await.py",
            r"async def f(self, session, run, hook, cleanup):
    try:
        async with session:
            error = None
            try:
                result = await run()
                if hook is not None:
                    await hook()
            except BaseException as caught:
                error = caught
                raise
            finally:
                if self.enabled:
                    try:
                        await session.stop()
                    except Exception:
                        if error is None:
                            raise
                        self.record(error)
    finally:
        await cleanup(session)
    return result
",
        ),
        (
            "async_manager_finally_attribute_linear_control.py",
            r"async def f(self, session, run, hook, cleanup):
    try:
        async with session:
            error = None
            try:
                result = await run()
                await hook()
            except BaseException as caught:
                error = caught
                raise
            finally:
                if self.enabled:
                    try:
                        await session.stop()
                    except Exception:
                        if error is None:
                            raise
                        self.record(error)
    finally:
        await cleanup(session)
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn guarded_recovery_finally_keeps_enclosing_manager_owners() {
    for (filename, source) in [
        (
            "manager_guarded_recovery_normal_finally.py",
            r"def target(manager, process):
    with manager:
        try:
            work()
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except Timeout:
                    process.kill()
                    process.wait()
",
        ),
        (
            "generator_manager_guarded_recovery_finally.py",
            r"def target(manager, process):
    with manager:
        try:
            yield process
            if process.poll() is not None:
                raise Failure()
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except Timeout:
                    process.kill()
                    process.wait()
",
        ),
        (
            "manager_guarded_linear_normal_finally_control.py",
            r"def target(manager, process):
    with manager:
        try:
            work()
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
",
        ),
        (
            "generator_manager_guarded_linear_finally_control.py",
            r"def target(manager, process):
    with manager:
        try:
            yield process
            if process.poll() is not None:
                raise Failure()
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
