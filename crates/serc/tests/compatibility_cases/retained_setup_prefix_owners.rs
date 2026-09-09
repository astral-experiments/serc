use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_retained_setup_original_prefix_owners() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for with_manager in [false, true] {
            for (joined, calls) in [
                (true, 2),
                (true, 3),
                (true, 4),
                (true, 7),
                (false, 4),
                (false, 7),
            ] {
                let body = format!(
                    "{}{}marker = None\ntry:\n    if value is not None:\n        {}consume(value)\n    else:\n        fallback()\nexcept Error:\n    recover()\n",
                    if joined {
                        "if flag:\n    prepare()\n"
                    } else {
                        ""
                    },
                    "work()\n".repeat(calls),
                    if matches!(kind, "coroutine" | "async_generator") {
                        "await "
                    } else {
                        ""
                    },
                );
                let body = if with_manager {
                    format!("with manager:\n{}", indent(&body))
                } else {
                    body
                };
                let source = format!(
                    "{}def f(value, flag, manager):\n{}{}",
                    if matches!(kind, "coroutine" | "async_generator") {
                        "async "
                    } else {
                        ""
                    },
                    if matches!(kind, "generator" | "async_generator") {
                        "    yield None\n"
                    } else {
                        ""
                    },
                    indent(&body),
                );
                assert_matches_cpython_marshal(
                    &source,
                    &format!("retained_setup_prefix_{kind}_{with_manager}_{joined}_{calls}.py"),
                );
            }
        }
    }
}

fn indent(source: &str) -> String {
    source.lines().fold(String::new(), |mut body, line| {
        body.push_str("    ");
        body.push_str(line);
        body.push('\n');
        body
    })
}
