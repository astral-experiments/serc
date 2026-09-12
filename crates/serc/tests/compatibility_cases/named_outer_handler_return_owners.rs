use crate::common;

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn matches_cpython_named_outer_handler_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
        ("async def", "yield value\n"),
    ] {
        for returned in ["", " value", " outer"] {
            if scope == "async def" && suspension == "yield value\n" && !returned.is_empty() {
                continue;
            }
            for prefix in ["", "record()\n"] {
                for following in ["", "finish()\n"] {
                    let handler = format!("{prefix}return{returned}\n");
                    let body = format!(
                        "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}{following}",
                        indent(&indent(&handler)),
                    );
                    let source = format!("{scope} f(value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(
                        &source,
                        "named_outer_handler_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_named_outer_handler_conditional_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
    ] {
        for returned in [" value", " outer"] {
            for prefix in ["", "record()\n"] {
                let handler = format!("{prefix}if enabled:\n    return{returned}\nfallback()\n");
                let body = format!(
                    "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}",
                    indent(&indent(&handler)),
                );
                let source = format!("{scope} f(value, enabled):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(
                    &source,
                    "named_outer_handler_conditional_return.py",
                );
            }
        }
    }
}

#[test]
fn preserves_named_outer_handler_scope_boundaries() {
    for source in [
        r"def f(value, enabled, manager):
    try:
        work()
    except Error:
        try:
            retry()
        except Other:
            return
",
        r"def f(value, enabled, manager):
    try:
        work()
    except Error as outer:
        try:
            retry()
        except Other as inner:
            return
",
        r"def f(value, enabled, manager):
    with manager:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
",
        r"def f(value, enabled, manager):
    try:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
    finally:
        cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "named_outer_scope_boundary.py");
    }
}

#[test]
fn matches_cpython_named_handler_teardown_before_intervening_finally() {
    for (scope, suspension) in [("def", ""), ("async def", "await ")] {
        for returned in ["inner", "None", "value"] {
            let source = format!(
                "{scope} example(value):\n    try:\n        {suspension}acquire()\n    except Exception as outer:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return {returned}\n        finally:\n            {suspension}close()\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_intervening_finally.py");
        }
        let source = format!(
            "{scope} control():\n    try:\n        {suspension}acquire()\n    except Exception:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return inner\n        finally:\n            {suspension}close()\n"
        );
        common::assert_matches_cpython_marshal(&source, "unnamed_outer_intervening_finally.py");
    }
}

#[test]
fn matches_cpython_manager_copied_handler_finally_slot_owners() {
    common::assert_matches_cpython_marshal(
        r"async def named_handler_nested_manager_return(manager, lock, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            if lock is not None:
                async with lock:
                    report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
",
        "named_handler_nested_manager_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def named_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
",
        "named_handler_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def unnamed_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception:
            report()
            return
        finally:
            if resource is not None:
                await resource.close()
",
        "unnamed_handler_return.py",
    );
}

#[test]
fn matches_cpython_copied_handler_finally_null_slot_with_iterators() {
    common::assert_matches_cpython_marshal(
        r"async def unnamed_handler_nested_loop_return(manager, second, resource, values):
    async with manager:
        async with second:
            for outer in values:
                for inner in values:
                    try:
                        await work()
                    except Exception:
                        report(outer, inner)
                        return
                    finally:
                        if resource is not None:
                            await resource.close()
",
        "unnamed_handler_nested_loop_return.py",
    );
}
