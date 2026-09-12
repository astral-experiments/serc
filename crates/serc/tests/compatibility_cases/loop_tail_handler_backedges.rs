use crate::common;

#[test]
fn matches_cpython_loop_tail_handler_backedges() {
    for prefix in ["", "async "] {
        for binding in ["", " as error"] {
            for returns in [false, true] {
                for loop_header in ["for item in items:", "while ready:"] {
                    for handler_tail in [
                        "if ready:\n    recover()",
                        "if ready:\n    recover()\nelse:\n    fallback()",
                        "if ready:\n    return recover()",
                    ] {
                        let returned = if returns { "return " } else { "" };
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            {returned}{awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_awaited_loop_tail_handler_backedges() {
    for binding in ["", " as error"] {
        for returns in [false, true] {
            for loop_header in ["for item in items:", "while ready:"] {
                let returned = if returns { "return " } else { "" };
                let source = format!(
                    "async def f(items, ready):\n    {loop_header}\n        try:\n            {returned}await work(item)\n        except Error{binding}:\n            record()\n            if ready:\n                await recover()\n    finish()\n",
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "awaited_loop_tail_handler_backedges.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_handler_backedges_with_else() {
    for prefix in ["", "async "] {
        for loop_header in ["for item in items:", "while ready:"] {
            for binding in ["", " as error"] {
                for handler_tail in [
                    "if ready:\n    recover(item)",
                    "if ready:\n    recover(item)\nelse:\n    fallback(item)",
                ] {
                    for normal_tail in [
                        "completed(item)",
                        "return item",
                        "if ready:\n    completed(item)",
                    ] {
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            result = {awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n        else:\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                            normal_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges_with_else.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_handler_interrupt_checks() {
    common::assert_matches_cpython_marshal(
        r"async def f(stop, wait, record, value):
    try:
        while True:
            try:
                await wait(stop)
                break
            except TimeoutError:
                value = stop.current()
                if value is not None:
                    record(value)
    except RuntimeError:
        pass
",
        "async_unnamed_loop.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(stop, wait, record, value):
    try:
        while True:
            try:
                wait(stop)
                break
            except TimeoutError:
                value = stop.current()
                if value is not None:
                    record(value)
    except RuntimeError:
        pass
",
        "sync_unnamed_loop.py",
    );
}

#[test]
fn matches_cpython_shared_protected_loop_interrupt_checks() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try:
            if item is None:
                continue
            else:
                consume(item)
                if ready(item):
                    emit(item)
        except Exception as exc:
            recover(exc)
",
        "protected_else_shared_tail.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(manager, items, limit=None):
    count = 0
    seen = 0
    errors = 0
    with manager as stream:
        for item in items:
            if limit is not None and seen >= limit:
                break
            seen += 1
            try:
                result = process(item)
                if result is None:
                    skipped(item)
                    errors += 1
                    continue
                else:
                    stream.write(result)
                    count += 1
                    if count % 100 == 0:
                        report(count)
            except Exception as exc:
                recover(exc)
    complete(errors)
",
        "manager_protected_else_shared_tail.py",
    );
}

#[test]
fn copied_assertion_backedges_leave_their_original_try() {
    for (source, filename) in [
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_tail_backedges.py",
        ),
        (
            "def f(items, verify, strict):\n    for item in items:\n        try:\n            work(item)\n            if verify:\n                if strict:\n                    assert check(item)\n                else:\n                    assert other(item)\n        except ValueError as error:\n            report(error)\n",
            "nested_assert_tail_backedges.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn final_else_copied_backedges_leave_nested_tries() {
    for (source, filename) in [
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_tail_backedges.py",
        ),
        (
            "def f(items, debug, flag):\n    for item in items:\n        try:\n            if not debug:\n                consume(item)\n            else:\n                try:\n                    work(item)\n                    if flag:\n                        consume(item)\n                except ValueError as error:\n                    report(error)\n                    pass\n        except RuntimeError as error:\n            report(error)\n            pass\n",
            "nested_try_final_else_backedges.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
