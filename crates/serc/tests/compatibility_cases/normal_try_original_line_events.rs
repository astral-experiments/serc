use crate::common;

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
