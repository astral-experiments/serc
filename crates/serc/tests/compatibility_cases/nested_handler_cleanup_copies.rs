use crate::common;

#[test]
fn matches_cpython_nested_handler_body_cleanup_copies() {
    for (header, prefix) in [
        ("def", ""),
        ("async def", "    await suspend()\n"),
        ("def", "    yield value\n"),
    ] {
        for body in ["result = work(value)", "work(value)"] {
            for following in ["return result", "finish(result)"] {
                for normal_else in ["", "        else:\n            result = other(value)\n"] {
                    let source = format!(
                        "{header} f(value):\n{prefix}    try:\n        result = work(value)\n    except ValueError:\n        try:\n            {body}\n        except TypeError:\n            raise\n{normal_else}    {following}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_handler_cleanup_copies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_else_join_controls() {
    for (header, prefix) in [
        ("def", ""),
        ("async def", "    await suspend()\n"),
        ("def", "    yield value\n"),
    ] {
        for body in [
            "            with manager():\n                result = work(value)\n",
            "            match value:\n                case 1:\n                    result = work(value)\n                case _:\n                    result = other(value)\n",
        ] {
            let source = format!(
                "{header} f(value):\n{prefix}    try:\n        result = work(value)\n    except ValueError:\n        try:\n{body}        except TypeError:\n            raise\n        else:\n            result = other(value)\n    return result\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_handler_else_joins.py");
        }
    }
}

#[test]
fn matches_cpython_nested_terminating_else_cleanup_locations() {
    for header in ["def", "async def"] {
        for prefix in ["", "                    prepare()\n"] {
            let source = format!(
                "{header} f(flag, outer, inner):\n    try:\n        work()\n    except Error as error:\n        if outer:\n            if inner:\n                action()\n            else:\n                if flag:\n{prefix}                    raise Other() from error\n        else:\n            raise\n    following()\n    return flag\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_terminating_else_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_nested_terminating_else_surviving_jumps() {
    for condition in ["flag", "not flag", "flag or other", "flag < other"] {
        for terminal in ["raise Other()", "raise", "return None"] {
            let source = format!(
                "def f(flag, outer, inner, other, manager, values):\n    if outer:\n        if inner:\n            action()\n        else:\n            if {condition}:\n                {terminal}\n    else:\n        raise Other()\n    following()\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_terminating_else_jump.py");
        }
    }
}

#[test]
fn keeps_nested_nonterminating_else_locations() {
    for body in [
        "                with manager:\n                    action()\n",
        "                try:\n                    action()\n                except Other:\n                    recover()\n",
    ] {
        let source = format!(
            "def f(flag, outer, inner, other, manager, values):\n    if outer:\n        if inner:\n            action()\n        else:\n            if flag:\n{body}    else:\n        raise Other()\n    following()\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_nonterminating_else_control.py");
    }
}

#[test]
fn preserves_nested_named_cleanup_copy_owners_at_loop_tail() {
    // The second POP_EXCEPT leaves the outer handler. Its copied tail belongs
    // to the scope fallback in an async function and has no owner in a normal one.
    for (header, suspend, inner_binding, error) in [
        ("def", "", "", "primary_error"),
        ("def", "", " as repair_error", "repair_error"),
        ("async def", "await ", " as repair_error", "repair_error"),
    ] {
        let source = format!(
            "{header} f(values, retry, skip):\n    errors = {{}}\n    pending = []\n    for value in values:\n        try:\n            primary(value)\n        except ValueError as primary_error:\n            if skip:\n                errors[value] = primary_error\n                if retry:\n                    pending.append(value)\n                continue\n            try:\n                {suspend}repair(value)\n            except TypeError{inner_binding}:\n                errors[value] = {error}\n                if retry:\n                    pending.append(value)\n    return pending\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "nested_named_cleanup_copy_owners_at_loop_tail.py",
        );
    }
}

#[test]
fn matches_cpython_fresh_condition_slot_in_nested_retained_try() {
    for header in ["def", "async def"] {
        let source = format!(
            "{header} f(items, service, enabled):\n    for item in items:\n        try:\n            if enabled:\n                try:\n                    removed = service.remove(item)\n                    if not removed:\n                        service.failed(item)\n                    else:\n                        replacement = service.create(item)\n                        if replacement is None:\n                            service.failed(item)\n                        else:\n                            service.ready(replacement)\n                except Exception as error:\n                    service.failed(error)\n        except Exception as error:\n            service.failed(error)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_retained_try_condition.py");
    }
}

#[test]
fn preserves_named_handler_return_owner_before_older_iterators() {
    for (filename, source) in [
        (
            "named_handler_return_before_outer_iterator.py",
            "def probe(values, run, retry, recover, close):\n    for value in values:\n        try:\n            try:\n                run(value)\n            except KeyError as error:\n                if retry(error):\n                    continue\n                return recover(error)\n        except RuntimeError:\n            run(value)\n        finally:\n            close(value)\n",
        ),
        (
            "named_handler_tuple_return_before_outer_iterator.py",
            "def probe(values, run, retry, recover, close):\n    for value in values:\n        try:\n            try:\n                run(value)\n            except KeyError as error:\n                if retry(error):\n                    continue\n                return recover(error), value\n        except RuntimeError:\n            run(value)\n        finally:\n            close(value)\n",
        ),
        (
            "named_handler_no_iterator_control.py",
            "def probe(value, run, recover, close):\n    try:\n        try:\n            run(value)\n        except KeyError as error:\n            return recover(error)\n    except RuntimeError:\n        run(value)\n    finally:\n        close(value)\n",
        ),
        (
            "named_handler_younger_iterator_control.py",
            "def probe(values, run, recover, close):\n    try:\n        try:\n            for value in values:\n                try:\n                    run(value)\n                except KeyError as error:\n                    return recover(error)\n        except RuntimeError:\n            run(None)\n    finally:\n        close(None)\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
