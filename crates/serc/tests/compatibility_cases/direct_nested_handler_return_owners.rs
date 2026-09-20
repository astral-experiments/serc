use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_direct_nested_handler_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
        ("async def", "yield value\n"),
    ] {
        for returned in ["", " value"] {
            if scope == "async def" && suspension == "yield value\n" && !returned.is_empty() {
                continue;
            }
            for prefix in ["", "record()\n"] {
                for following in ["", "finish()\n"] {
                    let handler = format!("{prefix}return{returned}\n");
                    let body = format!(
                        "{suspension}try:\n    work()\nexcept Error:\n    try:\n        retry()\n    except Other:\n{}{following}",
                        indent(&indent(&handler)),
                    );
                    let source = format!("{scope} f(value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(
                        &source,
                        "direct_nested_handler_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_direct_nested_handler_conditional_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
    ] {
        for returned in ["", " value"] {
            for prefix in ["", "record()\n"] {
                let handler = format!("{prefix}if enabled:\n    return{returned}\nfallback()\n");
                let body = format!(
                    "{suspension}try:\n    work()\nexcept Error:\n    try:\n        retry()\n    except Other:\n{}",
                    indent(&indent(&handler)),
                );
                let source = format!("{scope} f(value, enabled):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(
                    &source,
                    "direct_nested_handler_conditional_return.py",
                );
            }
        }
    }
}

#[test]
fn preserves_direct_nested_handler_scope_boundaries() {
    for source in [
        r"def f():
    try:
        work()
    except Error:
        with manager:
            try:
                retry()
            except Other:
                return
",
        r"def f():
    try:
        work()
    except Error:
        try:
            with manager:
                try:
                    retry()
                except Other:
                    return
        except Middle:
            recover()
",
        r"def f():
    try:
        work()
    except Error:
        try:
            retry()
        except Other as inner:
            return
",
        r"def f():
    try:
        work()
    except Error as outer:
        try:
            retry()
        except Other as inner:
            return
",
        r"def f():
    try:
        work()
    except Error:
        try:
            retry()
        except Other:
            try:
                return
            finally:
                cleanup()
",
        r"def f():
    try:
        try:
            work()
        except Error:
            try:
                retry()
            except Other:
                return
    finally:
        cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "nested_handler_scope_boundary.py");
    }
}

#[test]
fn matches_cpython_deeper_unnamed_handler_return_owners() {
    for handler_depth in [2, 3] {
        for returned in ["value", "None"] {
            let mut body = format!("return {returned}\n");
            for _ in 0..handler_depth {
                body = format!("try:\n    work()\nexcept Error:\n{}", indent(&body));
            }
            let source = format!("def f(value):\n{}", indent(&body));
            common::assert_matches_cpython_marshal(&source, "deeper_unnamed_handler_return.py");
        }
    }
}

#[test]
fn matches_cpython_inner_try_returns_through_unnamed_handler_chain() {
    for handler_depth in [2, 3] {
        for conditional in [false, true] {
            let returned = if conditional {
                "if flag:\n    return read(value)\n"
            } else {
                "return read(value)\n"
            };
            let mut body = format!("try:\n{}except LastError:\n    raise\n", indent(returned));
            for _ in 0..handler_depth {
                body = format!(
                    "try:\n    return read(value)\nexcept Error:\n{}",
                    indent(&body),
                );
            }
            let source = format!("def f(value, flag):\n{}", indent(&body));
            common::assert_matches_cpython_marshal(&source, "unnamed_handler_inner_try_return.py");
        }
    }
}

#[test]
fn matches_cpython_iterator_between_unnamed_handler_return_owners() {
    for (filename, source) in [
        (
            "unnamed_handler_iterator_inner_handler_none_return.py",
            r"def probe(items, operation):
    try:
        operation()
    except ValueError:
        for item in items:
            try:
                operation(item)
            except TypeError:
                return None
        return None
",
        ),
        (
            "unnamed_handler_iterator_inner_handler_value_return.py",
            r"def probe(items, operation, selected):
    try:
        operation()
    except ValueError:
        for item in items:
            try:
                operation(item)
            except TypeError:
                if selected(item):
                    return operation(item)
                continue
        return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_iterator_in_handler_try_value_return() {
    let source = r"async def probe(events, operation, result):
    try:
        try:
            await operation()
        except ValueError:
            try:
                async for item in events:
                    if item:
                        return result
            except TypeError:
                raise
    except RuntimeError:
        pass
    return result
";
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_handler_new_try_async_iterator_name_return.py",
    );
}
