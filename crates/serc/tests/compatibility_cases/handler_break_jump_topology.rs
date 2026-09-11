use crate::common;

#[test]
fn matches_cpython_shared_handler_break_jumps() {
    for prefix in [
        "",
        "            if tick():\n                await refresh()\n",
    ] {
        for protected in ["await task", "work()"] {
            for handler in ["pass", "report()"] {
                for body in ["await work()", "yield value"] {
                    for cleanup in [
                        "cleanup()",
                        "if pending():\n            await cleanup()",
                        "cleanup(task)",
                        "if task.done():\n            await cleanup(task)",
                    ] {
                        let source = format!(
                            "async def f(task, value):\n    try:\n        while True:\n{prefix}            if ready():\n                try:\n                    {protected}\n                except Error:\n                    {handler}\n                break\n            {body}\n    finally:\n        {cleanup}\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "shared_handler_break_jumps.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_overriding_return_after_with_breaks() {
    for (prefix, suspend) in [("", ""), ("async ", ""), ("", "    yield\n")] {
        for condition in ["True", "1"] {
            for footer in ["", "    finish()\n"] {
                let source = format!(
                    "{prefix}def f():\n{suspend}    try:\n        while {condition}:\n            with context():\n                break\n    finally:\n        return\n{footer}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "overriding_return_after_with_break.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_handler_break_through_an_enclosing_backedge() {
    for outer in ["for batch in batches:", "while ready():"] {
        for name in ["", " as error"] {
            for continuation in ["", "        finish_batch()\n"] {
                let source = format!(
                    "def f(batches):\n    {outer}\n        iterator = current_iterator()\n        while True:\n            try:\n                item = next(iterator)\n            except StopIteration{name}:\n                break\n            consume(item)\n{continuation}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "handler_break_through_outer_backedge.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_handler_break_from_finally_loop_inside_manager() {
    let source = r"def f(manager, values, enabled):
    with manager:
        try:
            work()
        finally:
            if enabled:
                for value in values:
                    try:
                        consume(value)
                    except LookupError:
                        break
                else:
                    try:
                        finish()
                    finally:
                        cleanup()
";
    common::assert_matches_cpython_marshal(source, "handler_break_inside_manager.py");
}

#[test]
fn matches_cpython_handler_break_that_exits_manager() {
    let source = r"def f(manager, values):
    for value in values:
        with manager:
            try:
                consume(value)
            except LookupError:
                break
    finish()
";
    common::assert_matches_cpython_marshal(source, "handler_break_exits_manager.py");
}

#[test]
fn matches_cpython_handler_break_through_ordinary_try_exit() {
    let source = r"def f(values):
    try:
        for value in values:
            try:
                if accepted(value):
                    return value
            except LookupError:
                break
    except RuntimeError:
        pass
    finish()
";
    common::assert_matches_cpython_marshal(source, "handler_break_ordinary_try_exit.py");
}

#[test]
fn matches_cpython_retained_finally_join_without_loop_break() {
    let source = r"def f(manager, values, enabled):
    with manager:
        try:
            work()
        finally:
            if enabled:
                for value in values:
                    try:
                        consume(value)
                    except LookupError:
                        pass
                else:
                    try:
                        finish()
                    finally:
                        cleanup()
";
    common::assert_matches_cpython_marshal(source, "retained_finally_join_without_break.py");
}

#[test]
fn matches_cpython_async_handler_break_through_two_try_exits() {
    let source = r"async def f(values, enabled):
    try:
        if enabled[0]:
            try:
                if enabled[1]:
                    for value in values:
                        await refresh(value)
                        try:
                            consume(value)
                        except LookupError:
                            break
            except RuntimeError:
                pass
    except RuntimeError:
        pass
    return None
";
    common::assert_matches_cpython_marshal(source, "async_handler_break_through_two_try_exits.py");
}

#[test]
fn matches_cpython_async_handler_break_through_one_try_exit() {
    let source = r"async def f(values, enabled):
    try:
        if enabled[0]:
            for value in values:
                await refresh(value)
                try:
                    consume(value)
                except LookupError:
                    break
    except RuntimeError:
        pass
    return None
";
    common::assert_matches_cpython_marshal(source, "async_handler_break_through_one_try_exit.py");
}

#[test]
fn matches_cpython_async_handler_break_through_three_try_exits() {
    let source = r"async def f(values, enabled):
    try:
        if enabled[0]:
            try:
                if enabled[1]:
                    try:
                        if enabled[2]:
                            for value in values:
                                await refresh(value)
                                try:
                                    consume(value)
                                except LookupError:
                                    break
                    except RuntimeError:
                        pass
            except RuntimeError:
                pass
    except RuntimeError:
        pass
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "async_handler_break_through_three_try_exits.py",
    );
}

#[test]
fn matches_cpython_handler_break_through_two_try_exits() {
    let source = r"def f(values, enabled):
    try:
        if enabled[0]:
            try:
                if enabled[1]:
                    for value in values:
                        refresh(value)
                        try:
                            consume(value)
                        except LookupError:
                            break
            except RuntimeError:
                pass
    except RuntimeError:
        pass
    return None
";
    common::assert_matches_cpython_marshal(source, "handler_break_through_two_try_exits.py");
}

#[test]
fn matches_cpython_async_handler_break_with_authored_pass_join() {
    let source = r"async def f(values, enabled):
    try:
        if enabled[0]:
            try:
                if enabled[1]:
                    for value in values:
                        await refresh(value)
                        try:
                            consume(value)
                        except LookupError:
                            break
                    pass
            except RuntimeError:
                pass
    except RuntimeError:
        pass
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "async_handler_break_with_authored_pass_join.py",
    );
}

#[test]
fn matches_cpython_call_constant_and_pass_original_block() {
    for (source, filename) in [
        (
            r"def f(enabled):
    try:
        if enabled: work(); 1; pass
    except Error:
        pass
    finish()
",
            "call_constant_pass_same_line.py",
        ),
        (
            r"def f(enabled):
    try:
        if enabled:
            work()
            1; pass
    except Error:
        pass
    finish()
",
            "call_constant_pass_next_line.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
