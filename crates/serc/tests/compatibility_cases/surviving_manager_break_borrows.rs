use crate::common;
use crate::common::indent;

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

#[test]
fn borrows_through_surviving_breaks_into_structured_manager_cleanup() {
    for manager in ["with", "async with"] {
        for enclosing_recovery in [false, true] {
            for cleanup in [
                "if value in pending:\n    await close(value)\n    pending.remove(value)\nfor item in pending:\n    item.close()\n",
                "try:\n    await close(value)\nexcept Exception as error:\n    report(error)\n    if other:\n        raise error\n",
                "try:\n    await close(value)\nexcept Exception:\n    raise\n",
            ] {
                let protected = "while True:\n    await work(value)\n    if flag:\n        break\n    if other:\n        value = work(value)\n        break\n";
                let managed = format!(
                    "{manager} manager:\n{}",
                    indent(&format!(
                        "try:\n{}finally:\n{}",
                        indent(protected),
                        indent(cleanup)
                    )),
                );
                let body = if enclosing_recovery {
                    format!(
                        "try:\n{}except RuntimeError:\n    report(value)\n",
                        indent(&managed)
                    )
                } else {
                    managed
                };
                let source = format!(
                    "async def function(manager, flag, other, value, pending):\n{}    return value\n",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "structured_manager_cleanup.py");
            }
        }
    }
}
