use crate::common;

#[test]
fn matches_cpython_normal_handler_cleanup_copies() {
    for (filename, source) in [
        (
            "normal_handler_cleanup_nested_explicit_else.py",
            r"def probe(work, a, b, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
            else:
                b()
    work()
",
        ),
        (
            "normal_handler_cleanup_nested_with.py",
            r"def probe(manager, work, a, b, flag, inner):
    with manager:
        try:
            work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    b()
",
        ),
        (
            "normal_handler_cleanup_nested_async_with_loop.py",
            r"async def probe(manager, work, a, b, flag, inner, items):
    async with manager:
        try:
            await work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    values = []
                    for item in items:
                        values.append(item)
                    b(values)
",
        ),
        (
            "normal_handler_cleanup_multi_statement_else.py",
            r"def probe(work, a, b, flag):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = a()
            b(value)
        else:
            value = b()
            a(value)
    work()
",
        ),
        (
            "normal_handler_cleanup_suspended_multi_else.py",
            r"async def probe(work, a, b, flag):
    try:
        await work()
    except ValueError:
        await work()
        if flag:
            value = await a()
            b(value)
        else:
            if flag:
                yield 1
            value = await a()
    else:
        b(0)
    b(1)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_conditional_normal_handler_cleanup_joins() {
    for (filename, source) in [
        (
            "normal_handler_cleanup_nested_no_else.py",
            r"def probe(work, a, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
    work()
",
        ),
        (
            "normal_handler_cleanup_multi_statement_no_else.py",
            r"def probe(work, a, b, flag):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = a()
            b(value)
    work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_cleanup_after_stable_tail() {
    for (filename, source) in [
        (
            "normal_cleanup_stable_tuple_runtime_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            left, right = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_comprehension_then_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            value = {item: item for item in items}
            first(value)
    work()
",
        ),
        (
            "normal_cleanup_stable_tuple_then_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            left, right = None, None
            first(left, right)
    work()
",
        ),
        (
            "normal_cleanup_stable_tuple_in_earlier_arm.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            left, right = None, None
        else:
            first()
            value = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_comprehension_in_earlier_arm.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = {item: item for item in items}
        else:
            first()
            value = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_nested_try_following_call.py",
            r"def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            try:
                result = work()
            except ValueError:
                first()
                result = first()
        else:
            first()
            result = first()
    first()
    return result
",
        ),
        (
            "normal_cleanup_stable_flat_small_return.py",
            r"def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            result = work()
        else:
            first()
            result = first()
    return result
",
        ),
        (
            "normal_cleanup_stable_simple_call_control.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            first()
            work()
    work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_manager_loop_return_fallback() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, items, ready):
    try:
        async with manager:
            if await ready():
                return 1
            async for item in items:
                if item:
                    return item
    except LookupError:
        return None
    except Exception:
        return None
    return None
",
        "async_manager_loop_return_fallback.py",
    );
}

#[test]
fn matches_cpython_async_manager_loop_finally_return_fallback() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, items, cleanup):
    try:
        async with manager:
            async for item in items:
                try:
                    if item is None:
                        continue
                    if item:
                        return item
                    return None
                finally:
                    try:
                        if item is not None:
                            cleanup(item)
                    except Exception:
                        pass
    except Exception:
        pass
    return None
",
        "async_manager_loop_finally_return_fallback.py",
    );
}

#[test]
fn preserves_original_cleanup_copy_targets_after_manager_elif() {
    let source = r#"def probe(manager, value, kind, action, report):
    try:
        if value is not None:
            if manager is None:
                return "no manager"
            with manager:
                state = action()
                if state == 1:
                    return "one"
                if state == 2:
                    return "two"
                action()
        elif kind == 1:
            action()
        else:
            report()
    except LookupError:
        return "missing"
    except OSError as error:
        return report(error)
    return "done"
"#;
    common::assert_matches_cpython_marshal(source, "cleanup_copy_manager_elif.py");
}

#[test]
fn preserves_original_cleanup_copy_targets_after_nested_try_elif() {
    let source = r"def probe(state, work, cleanup):
    try:
        if state == 0:
            if work():
                cleanup()
        elif state == 1:
            if work():
                cleanup()
        elif state == 2:
            try:
                if work():
                    cleanup()
                    work()
            except Exception:
                cleanup()
                raise
        elif state == 3:
            if work():
                cleanup()
        else:
            raise ValueError(state)
    except Exception:
        cleanup()
    return True
";
    common::assert_matches_cpython_marshal(source, "nested_try_elif_cleanup.py");
}

#[test]
fn matches_cpython_async_manager_and_pass_handler_cleanup_copies() {
    for (filename, source) in [
        (
            "named_pass_cleanup_control.py",
            r"def handle(flag, owner, work, send):
    try:
        work()
    except ValueError as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception as nested:
                pass
        else:
            send(str(error))
    except Exception as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception as nested:
                pass
        else:
            send(str(error))
",
        ),
        (
            "protected_async_manager_branch_implicit_return.py",
            r"async def consume(manager, items, write, managed):
    try:
        if managed:
            async with manager:
                async for item in items:
                    await write(item)
        else:
            async for item in items:
                await write(item)
    except RuntimeError:
        raise
    except BaseException:
        raise
",
        ),
        (
            "protected_async_manager_continuation_control.py",
            r"async def consume(manager, items, write, finish):
    try:
        async with manager:
            async for item in items:
                await write(item)
        await finish()
    except RuntimeError:
        raise
",
        ),
        (
            "unnamed_pass_cleanup_target.py",
            r"def handle(flag, owner, work, send):
    try:
        work()
    except ValueError as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception:
                pass
        else:
            send(str(error))
    except Exception as error:
        if flag:
            try:
                send(str(error))
                owner.closed = True
            except Exception:
                pass
        else:
            send(str(error))
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
