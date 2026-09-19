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
fn matches_cpython_named_outer_handler_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
        ("async def", "yield value\n"),
    ] {
        for returned in ["", " value", " outer"] {
            if scope == "async def" && suspension == "yield value\n" && !returned.is_empty() {
                continue;
            }
            for prefix in ["", "record()\n"] {
                for following in ["", "finish()\n"] {
                    let handler = format!("{prefix}return{returned}\n");
                    let body = format!(
                        "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}{following}",
                        indent(&indent(&handler)),
                    );
                    let source = format!("{scope} f(value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(
                        &source,
                        "named_outer_handler_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_named_outer_handler_conditional_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
    ] {
        for returned in [" value", " outer"] {
            for prefix in ["", "record()\n"] {
                let handler = format!("{prefix}if enabled:\n    return{returned}\nfallback()\n");
                let body = format!(
                    "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}",
                    indent(&indent(&handler)),
                );
                let source = format!("{scope} f(value, enabled):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(
                    &source,
                    "named_outer_handler_conditional_return.py",
                );
            }
        }
    }
}

#[test]
fn preserves_named_outer_handler_scope_boundaries() {
    for source in [
        r"def f(value, enabled, manager):
    try:
        work()
    except Error:
        try:
            retry()
        except Other:
            return
",
        r"def f(value, enabled, manager):
    try:
        work()
    except Error as outer:
        try:
            retry()
        except Other as inner:
            return
",
        r"def f(value, enabled, manager):
    with manager:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
",
        r"def f(value, enabled, manager):
    try:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
    finally:
        cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "named_outer_scope_boundary.py");
    }
}

#[test]
fn matches_cpython_named_handler_teardown_before_intervening_finally() {
    for (scope, suspension) in [("def", ""), ("async def", "await ")] {
        for returned in ["inner", "None", "value"] {
            let source = format!(
                "{scope} example(value):\n    try:\n        {suspension}acquire()\n    except Exception as outer:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return {returned}\n        finally:\n            {suspension}close()\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_intervening_finally.py");
        }
        let source = format!(
            "{scope} control():\n    try:\n        {suspension}acquire()\n    except Exception:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return inner\n        finally:\n            {suspension}close()\n"
        );
        common::assert_matches_cpython_marshal(&source, "unnamed_outer_intervening_finally.py");
    }
}

#[test]
fn matches_cpython_manager_copied_handler_finally_slot_owners() {
    common::assert_matches_cpython_marshal(
        r"async def named_handler_nested_manager_return(manager, lock, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            if lock is not None:
                async with lock:
                    report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
",
        "named_handler_nested_manager_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def named_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
",
        "named_handler_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def unnamed_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception:
            report()
            return
        finally:
            if resource is not None:
                await resource.close()
",
        "unnamed_handler_return.py",
    );
}

#[test]
fn matches_cpython_copied_handler_finally_null_slot_with_iterators() {
    common::assert_matches_cpython_marshal(
        r"async def unnamed_handler_nested_loop_return(manager, second, resource, values):
    async with manager:
        async with second:
            for outer in values:
                for inner in values:
                    try:
                        await work()
                    except Exception:
                        report(outer, inner)
                        return
                    finally:
                        if resource is not None:
                            await resource.close()
",
        "unnamed_handler_nested_loop_return.py",
    );
}

#[test]
fn matches_cpython_handler_return_through_enclosing_iterator_regions() {
    common::assert_matches_cpython_marshal(
        r#"def nested_handler_iterator_return(values):
    try:
        for value in values:
            try:
                try:
                    work(value)
                except Exception:
                    return str(value)
            except Exception:
                return str(value)
        return ""
    except Exception:
        return ""
"#,
        "nested_handler_iterator_return.py",
    );
}

#[test]
fn matches_cpython_finally_none_condition_reuses_retained_pop_prefix() {
    common::assert_matches_cpython_marshal(
        r"def copied_finally_prefix(value, first, second):
    try:
        consume(first, second)
        consume(first, second)
        consume(first, second)
        consume(first, second)
        return 0
    finally:
        if value is not None:
            finish(value)
",
        "copied_finally_prefix.py",
    );
    common::assert_matches_cpython_marshal(
        r"def normal_finally_prefix(value, first, second):
    try:
        consume(first, second)
        consume(first, second)
        consume(first, second)
        consume(first, second)
    finally:
        if value is not None:
            finish(value)
",
        "normal_finally_prefix.py",
    );
}

#[test]
fn matches_cpython_early_compaction_return_nop_cell() {
    common::assert_matches_cpython_marshal(
        r"def early_return_nop_owner(resource):
    try:
        consume(resource)
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)
",
        "early_return_nop_owner.py",
    );
    common::assert_matches_cpython_marshal(
        r"def early_return_nop_before_cell(resource):
    try:
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)
",
        "early_return_nop_before_cell.py",
    );
}

#[test]
fn matches_cpython_nested_fallback_handler_return_owners() {
    common::assert_matches_cpython_marshal(
        r"def f(ready):
    try:
        if ready:
            try:
                return primary()
            except ValueError:
                return fallback()
        finish()
    except Exception:
        recover()
",
        "fallback_handler_inner0_loop0.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, ready):
    for item in items:
        try:
            if ready:
                try:
                    return primary()
                except ValueError:
                    return fallback()
            finish()
        except Exception:
            recover()
",
        "fallback_handler_inner0_loop1.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(ready):
    try:
        if ready:
            try:
                return primary()
            except ValueError:
                try:
                    return fallback()
                except LookupError:
                    pass
        finish()
    except Exception:
        recover()
",
        "fallback_handler_inner1_loop0.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, ready):
    for item in items:
        try:
            if ready:
                try:
                    return primary()
                except ValueError:
                    try:
                        return fallback()
                    except LookupError:
                        pass
            finish()
        except Exception:
            recover()
",
        "fallback_handler_inner1_loop1.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f():
    try:
        return primary()
    except ValueError:
        try:
            return fallback()
        except LookupError:
            pass
",
        "fallback_handler_no_outer_try.py",
    );
}

#[test]
fn matches_cpython_two_fallback_handlers_inside_ordinary_try() {
    for value in ["0", "{}", "finish()"] {
        let source = format!(
            "def f():\n    try:\n        try:\n            return first()\n        except Exception:\n            try:\n                return second()\n            except Exception:\n                return {value}\n    except Exception:\n        return None\n"
        );
        common::assert_matches_cpython_marshal(&source, "two_fallback_handlers_outer_try.py");
    }
}

#[test]
fn matches_cpython_named_handler_exit_retains_inner_manager_owner() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager):
    try:
        with manager:
            try:
                await work()
            except Exception as error:
                report(error)
                record(error)
            return await finish()
    except Exception:
        return None
",
        "manager_handler_with_outer1.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(manager):
    with manager:
        try:
            await work()
        except Exception as error:
            report(error)
            record(error)
        return await finish()
",
        "manager_handler_with_outer0.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(manager):
    try:
        async with manager:
            try:
                await work()
            except Exception as error:
                report(error)
                record(error)
            return await finish()
    except Exception:
        return None
",
        "manager_handler_async_with_outer1.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(manager):
    async with manager:
        try:
            await work()
        except Exception as error:
            report(error)
            record(error)
        return await finish()
",
        "manager_handler_async_with_outer0.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(manager):
    with manager:
        try:
            try:
                await work()
            except Exception as error:
                report(error)
                record(error)
            return await finish()
        except Exception:
            return None
",
        "manager_before_outer_try.py",
    );
}

#[test]
fn matches_cpython_unmanaged_copied_handler_finally_slot_owners() {
    common::assert_matches_cpython_marshal(
        r"def close_on_exit(resource):
    try:
        work()
    except Error:
        report()
        return 0
    finally:
        if resource is not None:
            resource.close()
",
        "unmanaged_handler_finally_owner.py",
    );
    common::assert_matches_cpython_marshal(
        r"def close_on_exit(manager, resource):
    with manager:
        try:
            work()
        except Error:
            report()
            return 0
        finally:
            if resource is not None:
                resource.close()
",
        "managed_handler_finally_owner_control.py",
    );
    common::assert_matches_cpython_marshal(
        r"def close_on_exit(resource, report):
    try:
        work()
    except Error:
        report()
        return 0
    finally:
        if resource is not None:
            resource.close()
",
        "unmanaged_handler_finally_local_call_control.py",
    );
}

#[test]
fn matches_cpython_newer_try_named_handler_return_owners() {
    for (filename, source) in [
        (
            "newer_try_named_handler_constant_return.py",
            r"def run(first, second, flag):
    try:
        try:
            first()
        except LookupError as error:
            try:
                second()
                if flag:
                    return None
                return False
            except ValueError:
                raise
    except RuntimeError:
        return 1
",
        ),
        (
            "newer_try_named_handler_chain_returns.py",
            r"def run(first, second, third, fourth, flag):
    try:
        try:
            first()
        except LookupError as first_error:
            try:
                if flag:
                    return None
                return second()
            except ValueError as second_error:
                try:
                    return third()
                except KeyError as third_error:
                    try:
                        return fourth()
                    except RuntimeError as fourth_error:
                        return None
    except Exception:
        return False
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
