use crate::common::assert_matches_cpython_marshal;
use std::fmt::Write as _;

#[test]
fn matches_cpython_handler_exit_owners_across_jump_widths() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        for handler in ["Error", "Error as error", ""] {
            for outer in ["except", "finally", "both"] {
                for padding in [0, 30] {
                    let asynchronous = matches!(kind, "coroutine" | "async_generator");
                    let mut source = format!(
                        "{}def f(task, flag):\n",
                        if asynchronous { "async " } else { "" },
                    );
                    if matches!(kind, "generator" | "async_generator") {
                        source.push_str("    yield None\n");
                    }
                    source.push_str("    try:\n        if flag:\n            try:\n");
                    writeln!(
                        source,
                        "                {}",
                        match kind {
                            "coroutine" | "async_generator" => "await task",
                            "generator" => "yield task",
                            _ => "task()",
                        },
                    )
                    .unwrap();
                    if handler.is_empty() {
                        source.push_str("            except:\n");
                    } else {
                        writeln!(source, "            except {handler}:").unwrap();
                    }
                    source.push_str("                pass\n");
                    for _ in 0..padding {
                        source.push_str("        tick()\n");
                    }
                    source.push_str("        after()\n");
                    if matches!(outer, "except" | "both") {
                        source.push_str("    except OuterError:\n        recover()\n");
                    }
                    if matches!(outer, "finally" | "both") {
                        source.push_str("    finally:\n        cleanup()\n");
                    }
                    assert_matches_cpython_marshal(
                        &source,
                        &format!("handler_exit_width_{kind}_{handler}_{outer}_{padding}.py"),
                    );
                }
            }
        }
    }
}
