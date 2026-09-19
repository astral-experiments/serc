use std::fmt::Write as _;

use crate::common;

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        writeln!(result, "    {line}").expect("writing to a String cannot fail");
    }
    result
}

#[test]
fn borrows_through_surviving_breaks_into_manager_cleanup() {
    for (declaration, is_async, is_generator) in [
        ("def", false, false),
        ("async def", true, false),
        ("def", false, true),
        ("async def", true, true),
    ] {
        for manager in ["with", "async with"] {
            if manager == "async with" && !is_async {
                continue;
            }
            for cleanup in ["cleanup(owner)\n", "owner.waiting = False\n"] {
                for shape in ["break_continue", "no_continue", "direct_only"] {
                    let mut body = String::from(if is_async {
                        "item = await get()\n"
                    } else {
                        "item = get()\n"
                    });
                    if is_generator {
                        body.push_str("yield item\n");
                    }
                    if shape != "direct_only" {
                        body.push_str("if first(item):\n    break\n");
                    }
                    if shape == "break_continue" {
                        body.push_str("if repeat(item):\n    continue\n");
                    }
                    body.push_str("process(item)\nbreak\n");
                    let loop_body = format!("while True:\n{}", indent(&body));
                    let managed = format!("{manager} manager:\n{}", indent(&loop_body));
                    let returned = if is_async && is_generator {
                        "yield item\nreturn\n"
                    } else {
                        "return item\n"
                    };
                    let body = format!(
                        "try:\n{}finally:\n{}if isinstance(item, Problem):\n    raise item.error\n{returned}",
                        indent(&managed),
                        indent(cleanup),
                    );
                    let source =
                        format!("{declaration} function(owner, manager):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "surviving_manager_break.py");
                }
            }
        }
    }
}
