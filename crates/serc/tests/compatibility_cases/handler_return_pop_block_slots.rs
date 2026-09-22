use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_copied_finally_markers_after_unnamed_handler_returns() {
    for kind in ["coroutine", "generator", "async_generator"] {
        for value in ["", " None"] {
            if kind == "async_generator" && !value.is_empty() {
                continue;
            }
            for condition in ["value is None", "value is not None"] {
                let source = format!(
                    "{}def f(value, manager):\n    try:\n        work()\n    except Error:\n        return{value}\n    finally:\n        if {condition}:\n            cleanup(value)\n{}",
                    if kind == "generator" { "" } else { "async " },
                    if kind == "coroutine" {
                        ""
                    } else {
                        "    yield value\n"
                    },
                );
                assert_matches_cpython_marshal(&source, "handler_return_none_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_copied_finally_markers_after_handler_call_and_return() {
    for kind in ["coroutine", "generator", "async_generator"] {
        let source = format!(
            "{}def f(value, manager):\n    try:\n        work()\n    except Error:\n        work()\n        return\n    finally:\n        if value:\n            cleanup(value)\n{}",
            if kind == "generator" { "" } else { "async " },
            if kind == "coroutine" {
                ""
            } else {
                "    yield value\n"
            },
        );
        assert_matches_cpython_marshal(&source, "handler_call_return_finally.py");
    }
}

#[test]
fn matches_cpython_handler_cleanup_slots_with_preserved_values_and_names() {
    for asynchronous in [false, true] {
        for handler_name in ["", " as error"] {
            let source = format!(
                "{}def f(value, manager):\n    try:\n        work()\n    except Error{handler_name}:\n        return value\n    finally:\n        if value is None:\n            cleanup(value)\n",
                if asynchronous { "async " } else { "" },
            );
            assert_matches_cpython_marshal(&source, "handler_preserved_value_finally.py");
        }
    }
}

#[test]
fn matches_cpython_handler_return_slots_after_value_and_branch_boundaries() {
    for handler in [
        "        return (\n            value\n        )\n",
        "        if flag:\n            work()\n        return\n",
    ] {
        let source = format!(
            "async def f(value, flag, manager):\n    try:\n        work()\n    except Error:\n{handler}    finally:\n        if value is None:\n            cleanup(value)\n",
        );
        assert_matches_cpython_marshal(&source, "handler_value_branch_boundary.py");
    }
}

#[test]
fn matches_cpython_handler_conditional_returns_without_pending_finally() {
    assert_matches_cpython_marshal(
        "def f(value, flag):\n    try:\n        work()\n    except Error:\n        return value if flag else convert(value)\n",
        "handler_conditional_return_without_finally.py",
    );
}

#[test]
fn matches_cpython_named_handler_constant_return_cleanup_slots() {
    for binding in ["", " as exc"] {
        for returned in ["", " None", " 17", " value"] {
            for condition in ["caught is None", "caught is not None"] {
                let source = format!(
                    "def f(flag, value):\n    caught = None\n    try:\n        return get()\n    except Error{binding}:\n        caught = {}\n        if flag:\n            return{returned}\n    finally:\n        if {condition}:\n            cleanup(caught)\n            caught = None\n",
                    if binding.is_empty() { "error" } else { "exc" },
                );
                assert_matches_cpython_marshal(&source, "named_handler_constant_return_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_inner_handler_return_before_outer_except_and_finally() {
    for nested_while in [false, true] {
        let loop_header = if nested_while {
            "        while flag:\n"
        } else {
            ""
        };
        let indent = if nested_while {
            "            "
        } else {
            "        "
        };
        for binding in ["", " as error"] {
            for returned in ["", " value"] {
                let source = format!(
                    "def f(flag, value):\n    try:\n{loop_header}{indent}try:\n{indent}    work()\n{indent}except Inner{binding}:\n{indent}    return{returned}\n    except Outer:\n        recover()\n    finally:\n        cleanup()\n    finished()\n",
                );
                assert_matches_cpython_marshal(&source, "inner_handler_return_outer_try.py");
            }
        }
    }
}

#[test]
fn matches_cpython_inner_handler_return_preserves_copied_finally_marker_owners() {
    for kind in ["coroutine", "generator", "async_generator"] {
        let asynchronous = kind != "generator";
        let source = format!(
            "{}def f(flag, value):\n    try:\n        try:\n            {}work()\n        except Inner:\n            return\n    except Outer:\n        recover()\n    finally:\n        if flag is not None:\n            cleanup()\n{}",
            if asynchronous { "async " } else { "" },
            if asynchronous { "await " } else { "" },
            if kind == "coroutine" {
                ""
            } else {
                "    yield value\n"
            },
        );
        assert_matches_cpython_marshal(&source, "inner_handler_copied_finally_marker_owners.py");
    }
}

#[test]
fn matches_cpython_outer_try_owner_on_copied_handler_return_finally() {
    let source = r"def run():
    try:
        started = clock.read()
        try:
            work()
            return 0
        except Exception:
            failed()
            return 1
        finally:
            duration((clock.read() - started) * 1000)
    except Exception:
        return 1
";
    assert_matches_cpython_marshal(source, "outer_try_handler_return_finally.py");
}

#[test]
fn matches_cpython_outer_try_owner_on_named_handler_constant_finally() {
    for returned in [
        "", "False", "True", "None", "3", "'value'", "b'value'", "1.25",
    ] {
        for cleanup in ["finish()", "if path.exists():\n    path.remove()"] {
            let source = format!(
                "def f(path):\n    try:\n        try:\n            work()\n        except ValueError as error:\n            report(error)\n            return {returned}\n        finally:\n{}        finished()\n    except RuntimeError:\n        recover()\n",
                crate::common::indent(&crate::common::indent(&crate::common::indent(cleanup)))
            );
            assert_matches_cpython_marshal(&source, "named_handler_constant_finally_owner.py");
        }
    }
}

#[test]
fn matches_cpython_outer_try_owner_on_handler_raise_finally_control() {
    let source = r"def run():
    try:
        started = clock.read()
        try:
            work()
            return 0
        except Exception:
            failed()
            raise
        finally:
            duration((clock.read() - started) * 1000)
    except Exception:
        return 1
";
    assert_matches_cpython_marshal(source, "outer_try_handler_raise_finally_control.py");
}

#[test]
fn matches_cpython_two_named_handler_return_cleanup_before_finally() {
    let source = r"async def run():
    result = None
    try:
        try:
            await work()
        except Exception as outer_error:
            if recoverable(outer_error):
                try:
                    await recover()
                except Exception as inner_error:
                    if cancelled(inner_error):
                        raise
                    failed(inner_error)
                    result = make_result()
                    return result
            else:
                raise
        result = make_result()
        return result
    except Exception as run_error:
        failed(run_error)
        raise
    finally:
        finish()
        try:
            await publish(result)
        except Exception as publish_error:
            failed(publish_error)
            raise
";
    assert_matches_cpython_marshal(source, "two_named_handler_return_finally.py");
}

#[test]
fn matches_cpython_two_named_handler_raise_cleanup_before_finally_control() {
    let source = r"async def run():
    result = None
    try:
        try:
            await work()
        except Exception as outer_error:
            if recoverable(outer_error):
                try:
                    await recover()
                except Exception as inner_error:
                    if cancelled(inner_error):
                        raise
                    failed(inner_error)
                    result = make_result()
                    raise
            else:
                raise
        result = make_result()
        return result
    except Exception as run_error:
        failed(run_error)
        raise
    finally:
        finish()
        try:
            await publish(result)
        except Exception as publish_error:
            failed(publish_error)
            raise
";
    assert_matches_cpython_marshal(source, "two_named_handler_raise_finally_control.py");
}

#[test]
fn matches_cpython_value_return_through_while_and_two_tries() {
    for (filename, source) in [
        (
            "value_while_two_try_truth_finally.py",
            r"async def f(ready, value, resource):
    try:
        while ready:
            try:
                try:
                    return make_result(value=convert(value), success=True)
                except ValueError:
                    raise
            except Exception:
                raise
    finally:
        if resource:
            cleanup(resource)
",
        ),
        (
            "value_if_two_try_truth_finally_control.py",
            r"async def f(ready, value, resource):
    try:
        if ready:
            try:
                try:
                    return make_result(value=convert(value), success=True)
                except ValueError:
                    raise
            except Exception:
                raise
    finally:
        if resource:
            cleanup(resource)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_iterator_return_pop_runs() {
    for (filename, source) in [
        (
            "named_iterator_return_outer_try_none_finally.py",
            r#"async def f(items, result, resource):
    try:
        for item in items:
            try:
                await work(item)
            except Exception as error:
                if item:
                    result["error"] = f"failed: {error}"
                    return 1, result
    except Exception as error:
        return 2, result
    finally:
        if resource is not None:
            cleanup(resource)
"#,
        ),
        (
            "named_iterator_return_outer_try_truth_finally.py",
            r#"async def f(items, result, resource):
    try:
        for item in items:
            try:
                await work(item)
            except Exception as error:
                if item:
                    result["error"] = f"failed: {error}"
                    return 1, result
    except Exception as error:
        return 2, result
    finally:
        if resource:
            cleanup(resource)
"#,
        ),
        (
            "named_iterator_return_direct_finally_control.py",
            r#"async def f(items, result, resource):
    try:
        for item in items:
            try:
                await work(item)
            except Exception as error:
                if item:
                    result["error"] = f"failed: {error}"
                    return 1, result
    finally:
        if resource:
            cleanup(resource)
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_bare_return_finally_truth_slots() {
    for (filename, source) in [
        (
            "bare_return_global3_and_finally.py",
            r"async def f(stop, left, right):
    try:
        if stop:
            step_0()
            step_1()
            step_2()
            return
    finally:
        if left and right:
            cleanup()
",
        ),
        (
            "bare_return_global3_or_finally.py",
            r"async def f(stop, left, right):
    try:
        if stop:
            step_0()
            step_1()
            step_2()
            return
    finally:
        if left or right:
            cleanup()
",
        ),
        (
            "bare_return_global1_and_finally_control.py",
            r"async def f(stop, left, right):
    try:
        if stop:
            step_0()
            return
    finally:
        if left and right:
            cleanup()
",
        ),
        (
            "bare_return_global4_and_finally_control.py",
            r"async def f(stop, left, right):
    try:
        if stop:
            step_0()
            step_1()
            step_2()
            step_3()
            return
    finally:
        if left and right:
            cleanup()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_value_return_through_try_and_while() {
    for (filename, source) in [
        (
            "value_return_inside_try_while_finally_none_target.py",
            r#"async def cycle(metrics, events, watcher, completed=False, reason="end"):
    count = 0
    try:
        while True:
            event = await events.next()
            if event is None:
                return result(count=count, completed=False)
            count += 1
            if event.done:
                completed = True
                reason = None
                return result(count=count, completed=True)
    except Cancelled:
        reason = None
        raise
    except Exception:
        reason = "error"
        raise
    finally:
        if (
            metrics is not None
            and not completed
            and reason is not None
        ):
            metrics.finish(count, reason=reason)
        watcher.stop()
        if metrics is not None:
            metrics.flush()
"#,
        ),
        (
            "bare_return_inside_try_while_finally_none_control.py",
            r#"async def cycle(metrics, events, watcher, completed=False, reason="end"):
    count = 0
    try:
        while True:
            event = await events.next()
            if event is None:
                return
            count += 1
            if event.done:
                completed = True
                reason = None
                return
    except Cancelled:
        reason = None
        raise
    except Exception:
        reason = "error"
        raise
    finally:
        if (
            metrics is not None
            and not completed
            and reason is not None
        ):
            metrics.finish(count, reason=reason)
        watcher.stop()
        if metrics is not None:
            metrics.flush()
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_iterator_return_finally_truth_slots() {
    for (filename, source) in [
        (
            "yield_return_iterator_truth_negated.py",
            r"async def f(items, stop, ready):
    try:
        async for item in items:
            if stop:
                step_0()
                step_1()
                step_2()
                yield ready
                return
    finally:
        if not ready:
            cleanup()
",
        ),
        (
            "yield_return_iterator_truth_positive.py",
            r"async def f(items, stop, ready):
    try:
        async for item in items:
            if stop:
                step_0()
                step_1()
                step_2()
                yield ready
                return
    finally:
        if ready:
            cleanup()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_value_return_in_outer_loop_finally() {
    for (filename, source) in [
        (
            "value_return_outer_while_try_finally_none.py",
            r"async def f(items, ready, value, resource):
    while ready:
        try:
            if value:
                raise ValueError
            result = make_result(value=value)
            ready = not value
            return result
        except Exception:
            raise
        finally:
            if resource is not None:
                await cleanup(resource)
",
        ),
        (
            "value_return_outer_for_try_finally_none_control.py",
            r"async def f(items, ready, value, resource):
    for item in items:
        try:
            if value:
                raise ValueError
            result = make_result(value=value)
            ready = not value
            return result
        except Exception:
            raise
        finally:
            if resource is not None:
                await cleanup(resource)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_value_boolean_finally_cleanup_slots() {
    for (filename, source) in [
        (
            "named_handler_async_generator_boolean_call_finally.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record(error)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        suggestion = suggestion or rebuild()
"#,
        ),
        (
            "named_handler_async_generator_boolean_local_finally.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record(error)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        suggestion = replacement or suggestion
"#,
        ),
        (
            "named_handler_async_generator_explicit_call_guard_control.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record(error)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        if not suggestion:
            suggestion = rebuild()
"#,
        ),
        (
            "named_handler_async_generator_explicit_local_guard_control.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record(error)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        if replacement:
            suggestion = replacement
"#,
        ),
        (
            "named_handler_async_generator_fused_boolean_call_finally.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record("error", error, replacement)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        suggestion = suggestion or rebuild()
"#,
        ),
        (
            "named_handler_async_generator_fused_boolean_local_finally.py",
            r#"async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record("error", error, replacement)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        suggestion = replacement or suggestion
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_return_finalizers_with_enclosing_try_owners() {
    for (filename, source) in [
        (
            "nested_finalizer_outer_handler_computed_return.py",
            r#"def build_request(worker, manager, active, missing, timeout):
    async def run():
        try:
            token = None
            if active:
                token = worker.context.set("request")
            try:
                if missing:
                    return response(status=401, data={"error": "missing"})
                try:
                    with manager():
                        await worker.validate()
                except LookupError as error:
                    return response(status=error.status, data={"error": str(error)})
                with manager():
                    if timeout is None:
                        result = await worker.fetch()
                    else:
                        result = await worker.wait(timeout=float(timeout))
            finally:
                if token is not None:
                    worker.session.remove()
                    worker.replica().remove()
                    worker.context.reset(token)
        except TimeoutError as error:
            raise RuntimeError() from error
        except Exception as error:
            return response(status=500, data={"error": str(error)})
        return response(status=200, data=result)
    return run
"#,
        ),
        (
            "nested_finalizer_outer_handler_retry_return.py",
            r"async def poll(worker, retry):
    try:
        await worker.ready.wait()
        while True:
            finish = await worker.reserve()
            if finish is None:
                return
            try:
                if worker.abandoned:
                    await worker.cancel(finish=finish)
                    finish = None
                    return
                result = await worker.retrieve(finish=finish)
                if result.pending:
                    continue
                worker.commit(result, finish)
                finish = None
                return
            except CancelledError:
                raise
            except Exception as error:
                if retry(error):
                    continue
                await worker.cancel(error=error, finish=finish)
                finish = None
                return
            finally:
                if finish is not None:
                    finish.release()
    except CancelledError:
        await worker.cancel()
        raise
",
        ),
        (
            "nested_finalizer_no_outer_handler_computed_control.py",
            r#"def build_request(worker, manager, active, missing, timeout):
    async def run():
        token = None
        if active:
            token = worker.context.set("request")
        try:
            if missing:
                return response(status=401, data={"error": "missing"})
            try:
                with manager():
                    await worker.validate()
            except LookupError as error:
                return response(status=error.status, data={"error": str(error)})
            with manager():
                if timeout is None:
                    result = await worker.fetch()
                else:
                    result = await worker.wait(timeout=float(timeout))
        finally:
            if token is not None:
                worker.session.remove()
                worker.replica().remove()
                worker.context.reset(token)
        return response(status=200, data=result)
    return run
"#,
        ),
        (
            "nested_finalizer_no_outer_handler_retry_control.py",
            r"async def poll(worker, retry):
    await worker.ready.wait()
    while True:
        finish = await worker.reserve()
        if finish is None:
            return
        try:
            if worker.abandoned:
                await worker.cancel(finish=finish)
                finish = None
                return
            result = await worker.retrieve(finish=finish)
            if result.pending:
                continue
            worker.commit(result, finish)
            finish = None
            return
        except CancelledError:
            raise
        except Exception as error:
            if retry(error):
                continue
            await worker.cancel(error=error, finish=finish)
            finish = None
            return
        finally:
            if finish is not None:
                finish.release()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_bare_returns_through_two_try_cleanups() {
    for (filename, source) in [
        (
            "bare_handler_return_two_ordinary_tries_while.py",
            r"def example():
    try:
        while True:
            try:
                initialize()
                while True:
                    try:
                        message = read()
                        if not message:
                            continue
                        consume(message)
                    except KeyboardInterrupt:
                        record()
                        return
                    except Exception:
                        recover()
            except Exception:
                reset()
    except KeyboardInterrupt:
        finish()
",
        ),
        (
            "bare_handler_return_two_ordinary_tries_iterator.py",
            r"async def example(values):
    try:
        for value in values:
            try:
                if value:
                    try:
                        await operation(value)
                    except TimeoutError:
                        record(value)
                        return
                await consume(value)
            except Exception:
                recover(value)
    except Exception:
        finish()
",
        ),
        (
            "annotated_function_uncompacted_try_marker_target.py",
            r#"def example(reader):
    try:
        while True:
            try:
                initialize()
                while True:
                    try:
                        def prompt() -> str:
                            return "message"
                        message = reader.ask(prompt()).strip()
                        if not message:
                            continue
                        consume(message)
                    except KeyboardInterrupt:
                        record()
                        return
                    except Exception:
                        recover()
            except Exception:
                reset()
    except KeyboardInterrupt:
        finish()
"#,
        ),
        (
            "annotated_function_default_tuple_marker_control.py",
            r#"def example(reader):
    try:
        while True:
            try:
                initialize()
                while True:
                    try:
                        def prompt(value=0) -> str:
                            return "message"
                        message = reader.ask(prompt()).strip()
                        if not message:
                            continue
                        consume(message)
                    except KeyboardInterrupt:
                        record()
                        return
                    except Exception:
                        recover()
            except Exception:
                reset()
    except KeyboardInterrupt:
        finish()
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
