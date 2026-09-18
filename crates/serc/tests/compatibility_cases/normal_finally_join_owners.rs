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
fn matches_cpython_normal_finally_source_join_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for manager in ["with", "async with"] {
            if manager == "async with" && scope != "async def" {
                continue;
            }
            for condition in [
                "value is not None",
                "value is None",
                "value.attr",
                "value == enabled",
                "value in choices",
                "value and enabled",
            ] {
                let body = format!(
                    "try:\n    if enabled:\n        {suspension}\n    else:\n        other()\nfinally:\n    if {condition}:\n        {close}\n"
                );
                let body = format!("{manager} manager:\n{}", indent(&body));
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "normal_finally_source_join.py");
            }
        }
    }
}

#[test]
fn matches_cpython_normal_finally_structural_marker_owners() {
    for suspension in ["await work()", "yield value"] {
        for shape in ["expression", "iterator", "linear"] {
            let body = match shape {
                "expression" => "item = value if enabled else limit".to_string(),
                "iterator" => format!("for item in choices:\n    {suspension}"),
                _ if suspension == "yield value" => "yield value\nwork()\nwork()".to_string(),
                _ => suspension.to_string(),
            };
            for condition in ["value is not None", "value is None"] {
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        await close(value)\n",
                    indent(&body),
                );
                let body = format!("async with manager:\n{}", indent(&body));
                let source = format!(
                    "async def f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_structural_owner.py",
                );
            }
        }
    }
}

#[test]
fn preserves_normal_finally_original_prefix_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for prefix in ["work()\nwork()\n", "work()\nwork()\nwork()\n"] {
            for condition in ["value is not None", "value is None"] {
                let body = format!("if enabled:\n    {suspension}\nelse:\n    other()\n{prefix}");
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        {close}\n",
                    indent(&body),
                );
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_original_prefix.py",
                );
            }
        }
    }
}

#[test]
fn preserves_trailing_pass_before_synchronous_finally() {
    common::assert_matches_cpython_marshal(
        "def f(work, close):\n    try:\n        work()\n        pass\n    finally:\n        close()\n        close()\n",
        "trailing_pass_sync_finally.py",
    );
}

#[test]
fn preserves_trailing_pass_before_coroutine_finally() {
    common::assert_matches_cpython_marshal(
        "async def f(work, close):\n    await work()\n    try:\n        await work()\n        pass\n    finally:\n        await close()\n        await close()\n",
        "trailing_pass_coroutine_finally.py",
    );
}

#[test]
fn preserves_trailing_pass_before_generator_finally() {
    common::assert_matches_cpython_marshal(
        "def f(value, close):\n    try:\n        yield value\n        pass\n    finally:\n        close()\n        close()\n",
        "trailing_pass_generator_finally.py",
    );
}

#[test]
fn preserves_trailing_pass_before_async_generator_finally() {
    common::assert_matches_cpython_marshal(
        "async def f(value, close):\n    try:\n        yield value\n        pass\n    finally:\n        await close()\n        await close()\n",
        "trailing_pass_async_generator_finally.py",
    );
}

#[test]
fn reuses_trailing_pass_already_emitted_under_async_manager() {
    common::assert_matches_cpython_marshal(
        "async def f(manager, work, close):\n    async with manager:\n        try:\n            await work()\n            pass\n        finally:\n            await close()\n",
        "trailing_pass_async_manager_finally.py",
    );
}

#[test]
fn preserves_trailing_pass_before_finally_under_synchronous_manager() {
    common::assert_matches_cpython_marshal(
        "def f(manager, work, close):\n    with manager:\n        try:\n            work()\n            pass\n        finally:\n            close()\n",
        "trailing_pass_sync_manager_finally.py",
    );
}

#[test]
fn keeps_redundant_and_unreachable_trailing_passes_removed() {
    common::assert_matches_cpython_marshal(
        "def same_line(work, close):\n    try:\n        work(); pass\n    finally:\n        close()\n\ndef unreachable(close):\n    try:\n        return 1\n        pass\n    finally:\n        close()\n",
        "redundant_trailing_pass_finally.py",
    );
}

#[test]
fn retained_normal_finally_nop_inherits_through_original_empty_prefix() {
    for (scope, body) in [
        (
            "async def",
            "async for item in stream:\n    await work(item)",
        ),
        ("async def", "async for item in stream:\n    yield item"),
        (
            "async def",
            "async for item in stream:\n    yield item\ncompleted = True",
        ),
        ("def", "for item in stream:\n    work(item)"),
        ("def", "value = work()"),
        ("def", "yield stream\nvalue = work()"),
    ] {
        for cleanup in [
            "try:\n    close()\nexcept Error:\n    pass\n",
            "try:\n    close()\nfinally:\n    release()\n",
        ] {
            let body = format!(
                "try:\n{}except Error:\n    raise\nfinally:\n{}",
                indent(body),
                indent(cleanup),
            );
            let source = format!(
                "{scope} f(stream, work, close, release):\n{}",
                indent(&body),
            );
            common::assert_matches_cpython_marshal(&source, "retained_finally_empty_prefix.py");
        }
    }
}

#[test]
fn retained_normal_finally_nop_uses_the_else_exit_donor() {
    common::assert_matches_cpython_marshal(
        "def f(work, close, release):\n    try:\n        work()\n    except Error:\n        raise\n    else:\n        completed = True\n    finally:\n        try:\n            close()\n        finally:\n            release()\n",
        "retained_finally_else_exit.py",
    );
}

#[test]
fn shared_and_fallthrough_handler_finally_joins_keep_their_locations() {
    for (body, handler, cleanup) in [
        (
            "if enabled:\n    work()",
            "raise",
            "try:\n    close()\nfinally:\n    release()",
        ),
        (
            "for item in stream:\n    if item:\n        break\n    work()",
            "raise",
            "try:\n    close()\nfinally:\n    release()",
        ),
        (
            "value = work()",
            "recover()",
            "try:\n    close()\nfinally:\n    release()",
        ),
        ("value = work()", "raise", "close()"),
    ] {
        let body = format!(
            "try:\n{}except Error:\n{}finally:\n{}",
            indent(body),
            indent(handler),
            indent(cleanup),
        );
        let source = format!(
            "def f(enabled, stream, work, close, release, recover):\n{}",
            indent(&body),
        );
        common::assert_matches_cpython_marshal(&source, "guarded_finally_empty_prefix.py");
    }
}

#[test]
fn matches_cpython_membership_fold_after_retained_finally_pop() {
    for (filename, source) in [
        (
            "different_owner_worker_finally_multiline.py",
            r"def f(manager, pool, value, first, second):
    for attempt in range(3):
        try:
            with manager:
                worker = pool.acquire()
                try:
                    try:
                        worker.render(value, first, second)
                    except Error:
                        replacement = pool.replace(worker)
                        pool.release(replacement)
                        raise
                finally:
                    if worker in (
                        pool.workers
                    ):
                        pool.release(worker)
        except Error:
            retry(attempt)
",
        ),
        (
            "different_owner_worker_finally.py",
            r"def f(manager, pool, value, first, second):
    for attempt in range(3):
        try:
            with manager:
                worker = pool.acquire()
                try:
                    try:
                        worker.render(value, first, second)
                    except Error:
                        replacement = pool.replace(worker)
                        pool.release(replacement)
                        raise
                finally:
                    if worker in pool.workers:
                        pool.release(worker)
        except Error:
            retry(attempt)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_identity_fold_after_retained_finally_pop() {
    for (filename, source) in [
        (
            "identity_owner_worker_finally_multiline.py",
            r"def f(manager, pool, value, first, second):
    for attempt in range(3):
        try:
            with manager:
                worker = pool.acquire()
                try:
                    try:
                        worker.render(value, first, second)
                    except Error:
                        replacement = pool.replace(worker)
                        pool.release(replacement)
                        raise
                finally:
                    if worker is (
                        pool.workers
                    ):
                        pool.release(worker)
        except Error:
            retry(attempt)
",
        ),
        (
            "identity_owner_worker_finally.py",
            r"def f(manager, pool, value, first, second):
    for attempt in range(3):
        try:
            with manager:
                worker = pool.acquire()
                try:
                    try:
                        worker.render(value, first, second)
                    except Error:
                        replacement = pool.replace(worker)
                        pool.release(replacement)
                        raise
                finally:
                    if worker is not pool.workers:
                        pool.release(worker)
        except Error:
            retry(attempt)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn normal_finally_marker_range_end_preserves_null_pop_slot() {
    // Nine global-call folds place the marker in the erased POP_BLOCK's NULL cell.
    common::assert_matches_cpython_marshal(
        r#"async def f(manager, task, release, pool, values):
    with manager:
        try:
            await task.wait()
            if task.failed():
                raise ValueError("failed")
            print(f"count={len(values)} ready={values.count('ready')}")
            print(f"seen={len(values)} total={len(values)}")
            print(f"remaining={len(values)} final={len(values)}")
            print("complete")
        finally:
            release.set()
            if not task.done():
                task.cancel()
                await wait(task, return_exceptions=True)
            await pool.close()
"#,
        "normal_finally_nine_fold_pop_owner_target.py",
    );
}

#[test]
fn matches_cpython_named_retry_normal_truth_finalizer() {
    for (filename, source) in [
        (
            "coroutine_named_retry_break_normal_truth_finalizer.py",
            r"async def run(work, retry, items, report, finish):
    error = None
    try:
        await work()
    except ConnectionError as caught:
        error = caught
        if retry:
            for item in items:
                try:
                    await work()
                    error = None
                    return
                except ValueError as caught:
                    error = caught
                    await report(item)
                except Exception as caught:
                    error = caught
                    break
    except Exception as caught:
        error = caught
    finally:
        if error:
            await report(error)
        finish()
",
        ),
        (
            "coroutine_named_recovery_normal_truth_finalizer_control.py",
            r"async def run(work, report, finish):
    error = None
    try:
        await work()
    except Exception as caught:
        error = caught
    finally:
        if error:
            await report(error)
        finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
