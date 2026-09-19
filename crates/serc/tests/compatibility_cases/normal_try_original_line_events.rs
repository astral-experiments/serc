use crate::common;

#[test]
fn preserves_outer_condition_before_multiple_shared_return_paths() {
    for (first, second) in [
        ("value = first", "value = second"),
        ("first(value)", "second(value)"),
    ] {
        let source = format!(
            "def select(value, active, choose, first, second):\n    try:\n        if active:\n            if choose == 0:\n                {first}\n            elif choose == 1:\n                {second}\n            else:\n                raise ValueError()\n    except ValueError:\n        raise\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "outer_condition_shared_return.py");
    }
}

#[test]
fn preserves_single_successful_assignment_before_shared_return() {
    common::assert_matches_cpython_marshal(
        r"def repair(value, mismatch, enabled, replace, cleanup):
    try:
        if mismatch:
            changed = None
            if enabled:
                changed = replace()
            if changed is not None:
                value = changed
            else:
                raise ValueError()
    except Exception:
        try:
            cleanup()
        except Exception:
            pass
        raise
    return value
",
        "shared_assignment_return.py",
    );
}

#[test]
fn matches_cpython_shared_inner_try_exit_line_events() {
    for (name, tail) in [
        ("call", "if inner:\n    work(value)"),
        ("assignment", "if inner:\n    value = inner"),
        (
            "nested_conditions",
            "if inner:\n    if value:\n        work(value)",
        ),
    ] {
        let source = format!(
            "def f(outer, inner, value, work):\n    try:\n        if outer:\n            try:\n                {}\n            except ValueError:\n                pass\n    except Exception:\n        work(None)\n    work(value)\n    return value\n",
            tail.replace('\n', "\n                "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("shared_inner_try_exit_{name}.py"),
        );
    }
}

#[test]
fn matches_cpython_suspended_shared_inner_try_exit() {
    let source = r"async def f(outer, inner, value, work):
    try:
        if outer:
            try:
                await work(value)
                if inner:
                    await work(inner)
            except ValueError:
                await work(None)
                raise
    except Exception:
        await work(None)
    await work(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "suspended_shared_inner_try_exit.py");
}

#[test]
fn matches_cpython_borrows_after_shared_inner_try_exit() {
    let source = r"def f(outer, inner, value, work):
    try:
        value = work(value)
        if outer:
            try:
                outer = work(outer)
                if inner:
                    if value is None:
                        value = []
                    value.append(work(inner))
            except Exception:
                pass
    except Exception:
        return None
    if len(value) == 0:
        return None
    work(value)
    return [work(value), value, value]
";
    common::assert_matches_cpython_marshal(source, "borrows_after_shared_inner_try_exit.py");
}

#[test]
fn matches_cpython_original_normal_try_line_events() {
    for (filename, source) in [
        (
            "single_match_following_call.py",
            r"def f(value, work):
    try:
        match value:
            case 1:
                work()
    except Exception:
        work()
    work()
    return value
",
        ),
        (
            "match_two_literals_following_call.py",
            r"def f(value, work):
    try:
        match value:
            case 1:
                work()
            case 2:
                work()
    except Exception:
        work()
    work()
    return value
",
        ),
        (
            "conditional_inner_finally_following_call.py",
            r"def f(flag, work, recover):
    try:
        if flag:
            try:
                work()
            finally:
                recover()
    except Exception:
        recover()
    work()
    return flag
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_existing_normal_try_line_and_return_paths() {
    for (filename, source) in [
        (
            "conditional_inner_handler_following_call.py",
            r"def f(flag, work, recover):
    try:
        if flag:
            try:
                work()
            except ValueError:
                recover()
    except Exception:
        recover()
    work()
    return flag
",
        ),
        (
            "single_if_following_call.py",
            r"def f(flag, work):
    try:
        if flag:
            work()
    except Exception:
        work()
    work()
    return flag
",
        ),
        (
            "conditional_inner_handler.py",
            r"def f(flag, work, recover):
    try:
        if flag:
            try:
                work()
            except ValueError:
                recover()
    except Exception:
        recover()
    return flag
",
        ),
        (
            "conditional_many_calls.py",
            r"def f(flag, work, recover):
    try:
        if flag():
            work()
            work()
            try:
                work()
            except ValueError:
                recover()
    except Exception:
        recover()
    return work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_predecessors_at_normal_try_cleanup() {
    for (filename, source) in [
        (
            "conditional_constant_loop_cleanup.py",
            r"def f(enabled, stop, work, recover):
    try:
        if enabled:
            while True:
                if stop():
                    break
                work()
    except Exception:
        recover()
    work()
    return enabled
",
        ),
        (
            "async_manager_constant_loop_cleanup.py",
            r"async def f(manager, enabled, stop, work, recover):
    async with manager:
        try:
            if enabled:
                while True:
                    if stop():
                        break
                    await work()
        except Exception:
            recover()
        await work()
        return enabled
",
        ),
        (
            "conditional_constant_loop_else_cleanup.py",
            r"def f(enabled, stop, work, recover):
    try:
        if enabled:
            while True:
                if stop():
                    break
                work()
            else:
                work()
    except Exception:
        recover()
    work()
    return enabled
",
        ),
        (
            "conditional_dynamic_loop_cleanup.py",
            r"def f(enabled, condition, stop, work, recover):
    try:
        if enabled:
            while condition():
                if stop():
                    break
                work()
    except Exception:
        recover()
    work()
    return enabled
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_else_try_backedge_locations() {
    for (filename, source) in [
        (
            "async_terminal_else_try_backedge_target.py",
            r"async def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = await choose(item)
            if mode == 1:
                return await load(item)
            elif mode == 2:
                return await load(item)
            else:
                response = await load(item)
                if item not in response.failed:
                    return response
        except Exception as exc:
            record(exc)
    finish()
    return None
",
        ),
        (
            "sync_terminal_else_try_backedge_target.py",
            r"def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = choose(item)
            if mode == 1:
                return load(item)
            elif mode == 2:
                return load(item)
            else:
                response = load(item)
                if item not in response.failed:
                    return response
        except Exception as exc:
            record(exc)
    finish()
    return None
",
        ),
        (
            "async_terminal_else_try_linear_arm_control.py",
            r"async def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = await choose(item)
            if mode == 1:
                response = await load(item)
            elif mode == 2:
                return await load(item)
            else:
                response = await load(item)
                if item not in response.failed:
                    return response
        except Exception as exc:
            record(exc)
    finish()
    return None
",
        ),
        (
            "async_terminal_else_try_nonterminal_control.py",
            r"async def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = await choose(item)
            if mode == 1:
                return await load(item)
            elif mode == 2:
                return await load(item)
            else:
                response = await load(item)
                if item not in response.failed:
                    record(response)
        except Exception as exc:
            record(exc)
    finish()
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_original_pop_absorption_before_protected_raises() {
    for (filename, source) in [
        (
            "terminal_manager_assertion_pop_absorption_target.py",
            r"async def read_chunks(manager, failed):
    if failed:
        yield 'unavailable'
        return
    with manager() as stream:
        try:
            value = stream.read()
            assert value is not None
        except Exception as error:
            yield str(error)
            return
    yield value
",
        ),
        (
            "terminal_manager_assertion_shared_handler_control.py",
            r"async def read_chunks(manager, failed):
    if failed:
        yield 'unavailable'
        return
    with manager() as stream:
        try:
            value = stream.read()
            assert value is not None
        except Exception as error:
            yield str(error)
    yield value
",
        ),
        (
            "guarded_try_else_own_finally_pop_absorption_target.py",
            r"async def acquire_checked(connect, cleanup, active, key):
    try:
        value = await connect()
    except BaseException:
        raise
    else:
        if key not in active:
            await value.close()
            raise RuntimeError('inactive')
    finally:
        if key in active:
            cleanup(key)
    return value
",
        ),
        (
            "guarded_try_else_outer_finally_pop_absorption_target.py",
            r"def inspect_checked(acquire, cleanup, expected):
    try:
        prepare()
        try:
            value = acquire()
        except OSError:
            raise
        else:
            if value != expected:
                raise RuntimeError('changed')
    finally:
        if expected is not None:
            cleanup(expected)
    return value
",
        ),
        (
            "guarded_try_else_shared_finally_entry_control.py",
            r"def inspect_checked(acquire, cleanup, expected):
    try:
        prepare()
        try:
            value = acquire()
        except OSError:
            value = expected
        else:
            if value != expected:
                raise RuntimeError('changed')
    finally:
        if expected is not None:
            cleanup(expected)
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_located_normal_try_borrow_join_nops() {
    for (filename, source) in [
        (
            "normal_try_guard_terminal_recovery_nop_target.py",
            r"def checked_value(acquire, recover, finish):
    try:
        value = acquire()
        if value is None:
            raise ValueError('missing')
    except Exception as error:
        try:
            recover(error)
        except OSError:
            return None
        return False
    finish(value)
    return value
",
        ),
        (
            "protected_normal_try_guard_recovery_nop_target.py",
            r"def protected_step(check, recover, finish):
    try:
        try:
            if not check():
                raise ValueError('rejected')
        except Exception as error:
            try:
                recover()
            except OSError as recovery_error:
                raise RuntimeError('recovery failed') from error
            raise
        finish()
    except Exception as error:
        raise RuntimeError('failed') from error
",
        ),
        (
            "normal_try_shared_guard_recovery_nop_control.py",
            r"def checked_pair(first, second, recover, finish):
    try:
        if first() and second():
            raise ValueError('rejected')
    except Exception as error:
        try:
            recover(error)
        except OSError:
            return None
        return False
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_guarded_async_finally_empty_borrow_exit_nops() {
    for (filename, source) in [
        (
            "guarded_finally_empty_borrow_exit_async_target.py",
            r"async def guarded_cleanup(enabled, work, release, pack):
    result = None
    try:
        if enabled:
            try:
                result = await work()
            finally:
                try:
                    await release()
                except Exception:
                    pass
    except Exception:
        result = False
    return pack(result)
",
        ),
        (
            "guarded_finally_populated_borrow_exit_async_control.py",
            r"async def guarded_cleanup(enabled, work, release, pack):
    result = None
    try:
        if enabled:
            try:
                result = await work()
            finally:
                try:
                    await release()
                except Exception:
                    pass
            result = pack(result)
    except Exception:
        result = False
    return pack(result)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
