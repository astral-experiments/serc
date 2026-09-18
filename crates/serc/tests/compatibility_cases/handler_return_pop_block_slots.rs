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
