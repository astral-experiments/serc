use std::fmt::Write as _;

use crate::common;

#[test]
fn preserves_independent_entries_after_async_for_breaks() {
    for generator in [false, true] {
        for wrapper in ["plain", "if", "if_else"] {
            for body in [
                "if item:\n    break\n",
                "if item:\n    result = item\n    break\n",
                "if item.name == obj.name:\n    obj.value = item\n    break\n",
            ] {
                for consume_tail in [false, true] {
                    let mut source = String::from(
                        "async def function(values, flag, obj, consume):\n    result = flag\n",
                    );
                    if generator {
                        source.push_str("    yield flag\n");
                    }
                    let indent = if wrapper == "plain" {
                        "    "
                    } else {
                        "        "
                    };
                    if wrapper != "plain" {
                        source.push_str("    if flag:\n");
                    }
                    writeln!(source, "{indent}async for item in values:")
                        .expect("writing to a String cannot fail");
                    for line in body.lines() {
                        writeln!(source, "{indent}    {line}")
                            .expect("writing to a String cannot fail");
                    }
                    if wrapper == "if_else" {
                        source.push_str("    else:\n        result = flag\n");
                    }
                    if consume_tail {
                        source.push_str("    consume(result, flag)\n");
                    }
                    source.push_str(if generator {
                        "    yield result\n    return\n"
                    } else {
                        "    return result\n"
                    });
                    common::assert_matches_cpython_marshal(&source, "async_for_loop_entry.py");
                }
            }
        }
    }
}

#[test]
fn preserves_while_exit_borrows_reached_by_outer_branches() {
    // An outer condition can reach the live continuation directly even when the
    // loop's removed break edge stops at an empty original block.
    for scope in ["sync", "async", "gen", "agen"] {
        for wrapped in [false, true] {
            for direct_break in [false, true] {
                for call_before_return in [false, true] {
                    let mut body = String::from("while True:\n    consume(value)\n");
                    if direct_break {
                        body.push_str("    break\n");
                    } else {
                        body.push_str("    if done:\n        break\n");
                    }
                    if wrapped {
                        let mut outer = String::from("if flag:\n");
                        for line in body.lines() {
                            outer.push_str("    ");
                            outer.push_str(line);
                            outer.push('\n');
                        }
                        body = outer;
                    }
                    if call_before_return {
                        body.push_str("consume(value)\n");
                    }
                    if scope == "agen" {
                        body.push_str("yield value\nreturn\n");
                    } else {
                        body.push_str("return value\n");
                    }

                    let declaration = if matches!(scope, "async" | "agen") {
                        "async def"
                    } else {
                        "def"
                    };
                    let mut source = format!(
                        "{declaration} function(flag, done, value, other, consume, work):\n"
                    );
                    if matches!(scope, "gen" | "agen") {
                        source.push_str("    yield other\n");
                    }
                    if matches!(scope, "async" | "agen") {
                        source.push_str("    await work(other)\n");
                    }
                    for line in body.lines() {
                        source.push_str("    ");
                        source.push_str(line);
                        source.push('\n');
                    }
                    common::assert_matches_cpython_marshal(
                        &source,
                        "while_exit_live_outer_branch.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_async_iterator_headers_after_terminal_bodies() {
    for (prepare, iterable) in [
        ("", "iterator"),
        ("", "factory(page_size=1)"),
        (
            "client = service.get_container_client()\n",
            "client.list_blob_names(results_per_page=1)",
        ),
    ] {
        for prefix in [
            "",
            "if flag:\n    return None\n",
            "if flag:\n    action()\nelse:\n    other_action()\n",
        ] {
            for body in ["return item", "return True", "break", "consume(item)"] {
                let suite = format!(
                    "{prefix}{prepare}async for item in {iterable}:\n    {body}\nreturn None"
                );
                let source = format!(
                    "async def f(iterator, factory, service, flag):\n    {}\n",
                    suite.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "async_iterator_header_borrows.py");
            }
        }
    }
}

#[test]
fn protected_async_backedges_preserve_empty_finally_blocks() {
    for generator in [false, true] {
        for body in ["break", "assert item\nbreak", "await consume(item)\nbreak"] {
            for repeated in [false, true] {
                let mut source = String::from("async def f(manager, flag):\n");
                if generator {
                    source.push_str("    yield flag\n");
                }
                source.push_str("    try:\n");
                for _ in 0..if repeated { 2 } else { 1 } {
                    source.push_str("        async for item in manager.session():\n");
                    for line in body.lines() {
                        writeln!(source, "            {line}").unwrap();
                    }
                }
                source.push_str("    finally:\n        await manager.close()\n");
                common::assert_matches_cpython_marshal(&source, "protected_async_backedges.py");
            }
        }
    }
}

#[test]
fn named_conditional_handlers_preserve_normal_finally_reachability() {
    for scope in ["sync", "coroutine", "generator"] {
        for prefix in [false, true] {
            for cleanup in [
                "cleanup(value)",
                "obj.value = value",
                "if flag:\n    cleanup(value)",
            ] {
                for conditional_handler in [false, true] {
                    let mut source = String::from(if scope == "coroutine" {
                        "async def f(value, flag, obj):\n"
                    } else {
                        "def f(value, flag, obj):\n"
                    });
                    if scope == "generator" {
                        source.push_str("    yield value\n");
                    }
                    source.push_str("    try:\n        work(value)\n");
                    if conditional_handler {
                        source.push_str("    except Error as exc:\n");
                        if prefix {
                            source.push_str("        observe(value)\n");
                        }
                        source.push_str("        if flag:\n            raise\n");
                    } else {
                        source.push_str("    except Error:\n        raise\n");
                    }
                    source.push_str("    finally:\n");
                    for line in cleanup.lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    source.push_str("    consume(value)\n");
                    common::assert_matches_cpython_marshal(&source, "named_conditional_finally.py");
                }
            }
        }
    }
}

#[test]
fn protected_async_backedges_keep_surviving_conditional_breaks() {
    for generator in [false, true] {
        for exhaustion_else in [false, true] {
            for body in [
                "if flag:\n    continue\nif other:\n    break\nwork(item)\nbreak",
                "if flag:\n    break\nif other:\n    continue\nwork(item)\nbreak",
                "try:\n    work(item)\nexcept Exception:\n    pass\nif flag:\n    break\nwork(item)\nbreak",
                "if other:\n    continue\ntry:\n    work(item)\nexcept Exception:\n    pass\nif flag:\n    break\nwork(item)\nbreak",
                // A removed tail break alone still leaves the continuation unreachable.
                "if flag:\n    continue\nwork(item)\nbreak",
            ] {
                let mut source = String::from("async def f(values, value, flag, other):\n");
                if generator {
                    source.push_str("    yield other\n");
                }
                source.push_str("    try:\n        async for item in values:\n");
                for line in body.lines() {
                    writeln!(source, "            {line}").unwrap();
                }
                if exhaustion_else {
                    source.push_str("        else:\n            exhausted = True\n");
                }
                source.push_str("    finally:\n        work(value)\n    work(value)\n");
                common::assert_matches_cpython_marshal(&source, "protected_async_break_entries.py");
            }
        }
    }
}
