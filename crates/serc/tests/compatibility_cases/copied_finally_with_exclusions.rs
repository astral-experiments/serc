use crate::common;

#[test]
fn matches_cpython_copied_async_finally_exits_across_owner_fragments() {
    for body in [
        "return value",
        "return work(value)",
        "return await work(value)",
    ] {
        for prefix in ["", "        cleanup()\n"] {
            for inner in ["pass", "consume(value)", "await consume(value)"] {
                let source = format!(
                    "async def collect(value, manager):\n    try:\n        {body}\n    finally:\n{prefix}        async with manager:\n            {inner}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_async_finally_owner_fragments.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_comprehension_cleanup_after_conditional_finally_return() {
    for (prefix, prologue) in [("", ""), ("async ", ""), ("", "    yield None\n")] {
        for comprehension in [
            "[item for item in items if flag]",
            "{item for item in items if flag}",
            "{item: item for item in items if flag}",
        ] {
            let source = format!(
                "{prefix}def f(items, flag):\n{prologue}    try:\n        if flag:\n            return 1\n        return 2\n    finally:\n        result = {comprehension}\n        consume(result)\n"
            );
            common::assert_matches_cpython_marshal(&source, "conditional_finally_comprehension.py");
        }
    }
}

#[test]
fn matches_cpython_manager_exits_before_nested_finally_handlers() {
    for (definition, manager, suspension) in [
        ("def", "with", ""),
        ("async def", "with", "await "),
        ("async def", "async with", "await "),
    ] {
        for finalizer in [
            format!(
                "        try:\n            {suspension}finish()\n        except OSError:\n            {suspension}cleanup()\n"
            ),
            format!(
                "        try:\n            {suspension}finish()\n        finally:\n            {suspension}cleanup()\n"
            ),
        ] {
            let source = format!(
                "{definition} f(manager, work, finish, cleanup):
    try:
        {manager} manager:
            value = {suspension}work()
            return value
    finally:
{finalizer}"
            );
            common::assert_matches_cpython_marshal(&source, "copied_finally_with_exclusions.py");
        }
    }
}

#[test]
fn matches_cpython_outer_try_owns_manager_exit_after_inner_finally() {
    for (definition, manager, suspension) in [
        ("def", "with", ""),
        ("async def", "with", "await "),
        ("async def", "async with", "await "),
    ] {
        for returned in [
            "value".to_string(),
            "None".to_string(),
            format!("{suspension}work()"),
        ] {
            let source = format!(
                "{definition} f(manager, work, finish, value):
    try:
        {manager} manager:
            try:
                return {returned}
            finally:
                {suspension}finish()
    except OSError:
        return None
"
            );
            common::assert_matches_cpython_marshal(&source, "finally_before_manager_exit.py");
        }
    }
}

#[test]
fn matches_cpython_iterator_return_suppressed_finally_owners() {
    for (filename, source) in [
        (
            "iterator_return_call_finally_owner_control.py",
            r"def run(values, worker, cleanup):
    for value in values:
        try:
            result = worker(value)
            try:
                return value, result
            finally:
                cleanup.close()
        except LookupError:
            continue
    return None
",
        ),
        (
            "iterator_return_suppressed_finally_owner_target.py",
            r"def run(values, worker, cleanup):
    for value in values:
        try:
            result = worker(value)
            try:
                return value, result
            finally:
                try:
                    cleanup.close()
                except OSError:
                    pass
        except LookupError:
            continue
    return None
",
        ),
        (
            "outer_try_iterator_return_suppressed_finally_owner_control.py",
            r"def run(values, worker, cleanup):
    try:
        for value in values:
            result = worker(value)
            try:
                return value, result
            finally:
                try:
                    cleanup.close()
                except OSError:
                    pass
    except LookupError:
        return None
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_copied_finally_setup_nop_owners() {
    for (prefix, action) in [
        ("", "work()"),
        ("async ", "await work()"),
        ("", "yield work()"),
        ("async ", "yield work()"),
    ] {
        let source = format!(
            "{prefix}def f(flag, work):\n    try:\n        try:\n            {action}\n            return\n        finally:\n            if flag:\n                try:\n                    raise flag\n                finally:\n                    flag = None\n    except Error:\n        recover()\n"
        );
        common::assert_matches_cpython_marshal(&source, "copied_finally_setup_nop_owner.py");
    }
}
