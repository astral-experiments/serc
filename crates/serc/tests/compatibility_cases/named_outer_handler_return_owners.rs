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
