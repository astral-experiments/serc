use crate::common;
use crate::common::indent;

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

#[test]
fn matches_cpython_normal_finally_single_live_jump_setup_owner() {
    let source = r#"def complete(manager, condition, exists, remove, report):
    with manager():
        try:
            if condition():
                raise ValueError("invalid")
        except Exception:
            report()
            raise
        finally:
            try:
                if exists():
                    remove()
            except Exception:
                report()
"#;
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_single_live_jump_setup_owner.py",
    );
}

#[test]
fn matches_cpython_linear_finally_named_handler_return_owners() {
    for (filename, source) in [
        (
            "sync_named_handler_bare_return_finally_owner_target.py",
            r#"def run(worker, token, recover, fatal):
    try:
        try:
            worker.start(token)
        except LookupError as error:
            recover(error)
            if fatal:
                raise ValueError("failed") from error
            return
        except RuntimeError:
            recover(None)
            raise
        finally:
            worker.finish(token)
    except OSError:
        recover(None)
    return None
"#,
        ),
        (
            "live_named_handler_inner_finally_call_owner_target.py",
            r"def run(worker, cleanup, retry):
    try:
        try:
            return worker()
        except LookupError as error:
            if retry:
                try:
                    return worker()
                except OSError:
                    pass
                finally:
                    cleanup.finish(missing_ok=True)
            return worker()
    finally:
        cleanup.close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_finally_manager_join_after_raising_handlers() {
    for (filename, source) in [
        (
            "normal_finally_raising_handlers_manager_jump_owner.py",
            r#"def run(manager, cleanup, work, report):
    with manager:
        try:
            work()
        except LookupError as error:
            report(error)
            if cleanup is None:
                raise ValueError("missing") from error
            raise RuntimeError("lookup") from error
        except ValueError as error:
            report(error)
            raise RuntimeError("invalid") from error
        except Exception as error:
            raise RuntimeError("failed") from error
        finally:
            if cleanup is not None:
                cleanup.close()
                report("closed")
"#,
        ),
        (
            "normal_finally_raising_handlers_populated_exit_control.py",
            r#"def run(manager, cleanup, work, report):
    with manager:
        try:
            work()
        except LookupError as error:
            report(error)
            if cleanup is None:
                raise ValueError("missing") from error
            raise RuntimeError("lookup") from error
        except ValueError as error:
            report(error)
            raise RuntimeError("invalid") from error
        except Exception as error:
            raise RuntimeError("failed") from error
        finally:
            if cleanup is not None:
                cleanup.close()
                report("closed")
            report("finished")
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_protected_manager_match_and_nested_finally_joins() {
    for (filename, source) in [
        (
            "sync_manager_protected_match_failure_join_target.py",
            r"async def protected_match(manager, value):
    with manager:
        try:
            match value:
                case First():
                    await handle_first(value)
                case Second():
                    await handle_second(value)
        except Exception as error:
            record(error)
            raise
    finish()
",
        ),
        (
            "sync_manager_generator_nested_finally_join_target.py",
            r"def nested_cleanup(manager, resource, lease):
    with manager:
        try:
            yield
        finally:
            try:
                restore()
            finally:
                try:
                    if resource is not None:
                        resource.close()
                finally:
                    if lease is not None:
                        try:
                            unlock(lease)
                        finally:
                            close(lease)
",
        ),
        (
            "sync_manager_generator_nested_finally_following_call_control.py",
            r"def nested_cleanup(manager, resource, lease):
    with manager:
        try:
            yield
        finally:
            try:
                restore()
            finally:
                try:
                    if resource is not None:
                        resource.close()
                finally:
                    if lease is not None:
                        try:
                            unlock(lease)
                        finally:
                            close(lease)
        after_protected()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_generator_and_nested_raising_manager_join_owners() {
    for (filename, source) in [
        (
            "async_generator_manager_normal_finally_join_owner_target.py",
            r"async def stream(manager, values, result, emit, cleanup, restore):
    try:
        with manager():
            try:
                async for value in values():
                    yield value
            except Exception:
                for value in await cleanup():
                    yield value
                raise
            finally:
                emit()
                if result is not None:
                    emit(result)
                    await cleanup()
    finally:
        restore()
",
        ),
        (
            "nested_conditional_raising_finally_manager_join_owner_target.py",
            r"def run(first, second, third, work, close, collect, report, state):
    with first(), second(), third():
        try:
            work()
        finally:
            failures = collect()
            try:
                close()
            except Exception as error:
                failures.append(error)
            if failures:
                if state[0] is not None:
                    report(errors=[repr(error) for error in failures])
                else:
                    raise RuntimeError(failures)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_trailing_pass_after_compound_try_body() {
    for definition in ["def", "async def"] {
        for body in [
            "if flag:\n    work()",
            "with manager():\n    work()",
            "try:\n    work()\nexcept Error:\n    other()",
        ] {
            let source = format!(
                "{definition} f(flag):\n    try:\n        {}\n        pass\n    finally:\n        cleanup()\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "compound_try_body_trailing_pass.py");
        }
    }
}

#[test]
fn normal_finally_comparisons_reuse_their_original_owned_slots() {
    for (scope, suspension) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
        ("async def", "yield await work()"),
    ] {
        for protected in [
            format!("while value:\n    {suspension}\n"),
            format!("try:\n    {suspension}\nexcept Error as error:\n    raise error\n"),
        ] {
            for comparison in ["value > 0", "value == limit", "state.inner.value == ACTIVE"] {
                let body = format!(
                    "try:\n{}finally:\n    if {comparison}:\n        clean()\n",
                    indent(&protected),
                );
                let source = format!(
                    "{scope} f(manager, value, limit, state):\n    with manager:\n{}",
                    indent(&indent(&body)),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_comparison_slot.py",
                );
            }
        }
    }
}
