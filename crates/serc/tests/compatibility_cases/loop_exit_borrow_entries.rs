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

#[test]
fn protected_async_conditional_tail_break_keeps_following_loads_owned() {
    let source = r"async def f(items, client, count):
    try:
        async for item in items:
            consume(item)
            count += 1
            if count >= 10:
                break
        if count == 0:
            consume(count)
    finally:
        await client.close()
";
    common::assert_matches_cpython_marshal(source, "async_conditional_tail_break.py");
}

#[test]
fn protected_async_conditional_tail_break_with_work_keeps_cleanup_owned() {
    let source = r"async def f(items, client):
    try:
        async for item in items:
            if item.ready:
                consume(item)
            if item.done:
                consume(item)
                break
    finally:
        await client.close()
";
    common::assert_matches_cpython_marshal(source, "async_conditional_tail_break_work.py");
}

#[test]
fn protected_async_conditional_break_before_work_keeps_live_cleanup() {
    let source = r"async def f(items, client, flag):
    try:
        async for item in items:
            if flag:
                break
            consume(item)
    finally:
        await client.close()
";
    common::assert_matches_cpython_marshal(source, "async_conditional_break_before_work.py");
}

#[test]
fn protected_async_conditional_break_with_else_keeps_live_cleanup() {
    let source = r"async def f(items, client, flag):
    try:
        async for item in items:
            if flag:
                break
            else:
                consume(item)
    finally:
        await client.close()
";
    common::assert_matches_cpython_marshal(source, "async_conditional_break_with_else.py");
}

#[test]
fn named_guard_with_linear_recovery() {
    common::assert_matches_cpython_marshal(
        r"def f(value, local):
    try:
        result = work(value)
    except Error as error:
        if local:
            raise error
        report(error)
        result = []
    finally:
        if not local:
            sync()
            finish()
    return result
",
        "named_guard_linear_recovery.py",
    );
}

#[test]
fn named_guard_with_protected_for() {
    common::assert_matches_cpython_marshal(
        r"def f(items, local):
    try:
        for item in items:
            result = work(item)
    except Error as error:
        if local:
            raise error
        report(error)
        result = []
    finally:
        if not local:
            sync()
            finish()
    return result
",
        "named_guard_protected_for.py",
    );
}

#[test]
fn borrows_after_async_manager_break_recovery() {
    let source = r"async def f(manager, work, value):
    processed = False
    while True:
        try:
            with manager as item:
                if item is None:
                    break
                processed = True
                await work(item)
        except Error as exc:
            if isinstance(exc, Stop):
                raise
            await backoff(value)
            break
    if not processed:
        await backoff(value)
";
    common::assert_matches_cpython_marshal(source, "async_manager_break_recovery.py");
}

#[test]
fn borrows_after_nested_async_manager_break_recovery() {
    let source = r"async def f(manager, work, value):
    while True:
        try:
            processed = False
            while True:
                try:
                    with manager as item:
                        if item is None:
                            break
                        processed = True
                        await work(item)
                except Error as exc:
                    if isinstance(exc, Stop):
                        raise
                    await backoff(value)
                    break
            if not processed:
                await backoff(value)
        except Stop:
            raise
        except Error:
            await backoff(value)
";
    common::assert_matches_cpython_marshal(source, "nested_async_manager_break_recovery.py");
}

#[test]
fn nested_async_break_preserves_owned_finalizer_receivers() {
    common::assert_matches_cpython_marshal(
        r"async def f(client, terminal, worker, events, value, choose, emit):
    try:
        async with client:
            with terminal:
                worker.start()
                try:
                    async for event in events:
                        terminal.update(event)
                        if event.changed:
                            status = event.status
                            if choose(status if isinstance(status, str) else None):
                                break
                finally:
                    worker.cancel()
                    await worker.wait()
            emit(value.first)
            emit(value.second)
    finally:
        await worker.stop()
    emit(value.third)
",
        "nested_async_break_finalizer_receivers.py",
    );
}

#[test]
fn nested_async_break_reconverges_call_argument_conditions() {
    for condition in ["choose(event)", "choose(event if ready else None)"] {
        let source = format!(
            "async def f(events, worker, ready, choose):\n    try:\n        async for event in events:\n            if ready:\n                if {condition}:\n                    break\n    finally:\n        worker.cancel()\n        await worker.wait()\n    emit(worker)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_async_break_call_condition.py");
    }
}

#[test]
fn nested_async_break_retains_earlier_break_control() {
    assert_nested_async_break_exit_control(
        "if ready:\n                break\n            if choose(event):\n                if flag:\n                    break",
    );
}

#[test]
fn nested_async_break_retains_explicit_else_control() {
    assert_nested_async_break_exit_control(
        "if ready:\n                if choose(event):\n                    break\n                else:\n                    emit(event)",
    );
}

#[test]
fn nested_async_break_retains_nonterminal_work_control() {
    assert_nested_async_break_exit_control(
        "if ready:\n                if choose(event):\n                    break\n            emit(event)",
    );
}

#[test]
fn nested_async_break_preserves_conditional_expression_entry() {
    assert_nested_async_break_exit_control(
        "if ready:\n                if choose(event) if flag else ready:\n                    break",
    );
}

#[test]
fn nested_async_break_preserves_boolean_expression_entry() {
    assert_nested_async_break_exit_control(
        "if ready:\n                if flag and choose(event):\n                    break",
    );
}

fn assert_nested_async_break_exit_control(body: &str) {
    let source = format!(
        "async def f(events, worker, ready, choose, flag):\n    try:\n        async for event in events:\n            {body}\n    finally:\n        worker.cancel()\n        await worker.wait()\n    emit(worker)\n"
    );
    common::assert_matches_cpython_marshal(&source, "nested_async_break_exit_controls.py");
}

#[test]
fn protected_async_final_elif_break_keeps_following_loads_owned() {
    common::assert_matches_cpython_marshal(
        r"async def f(events, worker, ready, choose):
    try:
        async for event in events:
            if ready:
                worker.update(event)
            elif choose(event):
                break
    finally:
        worker.cancel()
        await worker.wait()
    emit(worker)
",
        "protected_async_final_elif_break.py",
    );
}

#[test]
fn protected_async_completed_prefix_keeps_following_loads_owned() {
    common::assert_matches_cpython_marshal(
        r"async def f(events, worker, ready, choose):
    try:
        async for event in events:
            if ready:
                if event.changed:
                    worker.update(event)
                if choose(event):
                    break
    finally:
        worker.cancel()
        await worker.wait()
    emit(worker)
",
        "protected_async_nested_prefix_break.py",
    );
}

#[test]
fn protected_async_completed_break_prefixes_preserve_borrow_reachability() {
    for (source, filename) in [
        (
            r"async def f(stream, value, should_stop, should_log, should_retry, consume):
    try:
        async for item in stream:
            if should_stop(item):
                if should_log(item):
                    consume(item)
                    if value:
                        consume(value)
                if should_retry(item):
                    value = item
                    continue
                value = item
                break
        consume(value)
        consume(value)
    except RuntimeError:
        consume(value)
",
            "async_break_after_completed_if_continue.py",
        ),
        (
            r"async def f(stream, values, value, should_stop, should_finish, consume):
    try:
        async for item in stream:
            consume(item)
            if should_stop(item):
                value = item
                for inner in values:
                    consume(inner)
                if should_finish(item):
                    value = item
                    break
        consume(value)
        consume(value)
    finally:
        consume(value)
",
            "async_break_after_completed_for.py",
        ),
        (
            r"async def f(stream, values, value, should_stop, should_finish, consume):
    try:
        async for item in stream:
            if should_finish(item):
                break
            consume(item)
            if should_stop(item):
                for inner in values:
                    consume(inner)
                if should_finish(value):
                    value = item
                    break
        consume(value)
        consume(value)
    finally:
        consume(value)
",
            "async_earlier_break_survives_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
