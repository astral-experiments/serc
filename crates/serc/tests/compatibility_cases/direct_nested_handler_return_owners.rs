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
