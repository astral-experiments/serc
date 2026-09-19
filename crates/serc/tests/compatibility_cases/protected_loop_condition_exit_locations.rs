use crate::common;

#[test]
fn matches_cpython_protected_loop_condition_exit_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for loop_header in ["for item in items:", "while active:"] {
            for condition in [
                "a or b",
                "not (a and b)",
                "(a and b) or c",
                "a and b",
                "False or a",
                "a < b < c",
            ] {
                for name in ["", " as error"] {
                    let source = format!(
                        "{function} f(items, active, a, b, c):\n{suspension}    {loop_header}\n        try:\n            if {condition}:\n                work()\n        except Exception{name}:\n            recover()\n    after()\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "protected_loop_condition_exit_locations.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_finally_backedges_after_returning_try_handlers() {
    for (function, suspension) in [("def", ""), ("async def", ""), ("def", "    yield seed\n")] {
        for loop_header in ["for item in items:", "while flag:"] {
            for name in ["", " as error"] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n{suspension}    {loop_header}\n        try:\n            return seed\n        except Error{name}:\n            recover(seed)\n        finally:\n            if flag:\n                cleanup(seed)\n    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_finally_backedges_after_structured_cleanup() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield seed\n"),
        ("async def", "    yield seed\n"),
    ] {
        for loop_header in ["for item in items:", "while flag:"] {
            for finalizer in [
                "            prepare(seed)\n            if flag:\n                cleanup(seed)\n",
                "            if flag:\n                cleanup(seed)\n                finish(seed)\n",
                "            if flag:\n                if other:\n                    cleanup(seed)\n",
            ] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n{suspension}    {loop_header}\n        try:\n            work(seed)\n        finally:\n{finalizer}    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_finally_manager_cleanup_joins() {
    for (function, suspension, manager_kind) in [
        ("def", "", "with"),
        ("async def", "", "async with"),
        ("def", "    yield seed\n", "with"),
        ("async def", "    yield seed\n", "async with"),
    ] {
        for (before, after) in [
            ("", ""),
            ("                prepare(seed)\n", ""),
            ("", "                finish(seed)\n"),
        ] {
            let source = format!(
                "{function} f(items, flag, other, seed, manager):\n{suspension}    for item in items:\n        try:\n            work(seed)\n        finally:\n            if flag:\n{before}                {manager_kind} manager:\n                    cleanup(seed)\n{after}    done(seed)\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_try_else_backedge_reachability() {
    for function in ["def", "async def"] {
        for loop_header in ["for item in items:", "while flag:"] {
            for try_body in ["return seed", "work(seed)"] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n    {loop_header}\n        try:\n            {try_body}\n        except Error:\n            recover(seed)\n        else:\n            if flag:\n                cleanup(seed)\n    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_for_boolean_discard_locations_in_suspended_functions() {
    for (function, suspension) in [("async def", ""), ("def", "    yield None\n")] {
        for body in [
            "        (\n            left(item)\n            and right(item)\n        )\n",
            "        first(item) or second(item) or third(item)\n",
            "        left(item) and (middle(item) or right(item))\n",
        ] {
            let source = format!(
                "{function} f(items, flag, item):\n{suspension}    for item in items:\n{body}    done()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }

    common::assert_matches_cpython_marshal(
        "async def f(items, flag, item):\n    yield None\n    for item in items:\n        (left(item)\n         and right(item))\n    done()\n",
        "protected_loop_condition_exit_locations.py",
    );
}

#[test]
fn matches_cpython_loop_boolean_discard_boundaries() {
    for (function, suspension, loop_header) in [
        ("def", "", "while flag:"),
        ("def", "", "while True:"),
        ("async def", "", "async for item in items:"),
        ("def", "    yield None\n", "while True:"),
    ] {
        for body in [
            "        left(item) and right(item)\n        consume(item)\n",
            "        result = left(item) and right(item)\n",
        ] {
            let source = format!(
                "{function} f(items, flag, item):\n{suspension}    {loop_header}\n{body}    done()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_folded_loop_boolean_discard_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n"),
    ] {
        let source = format!(
            "{function} f(items):\n{suspension}    sentinel = False\n    for item in items:\n        (False or True) and (True or False)\n    done()\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "protected_loop_condition_exit_locations.py",
        );
    }
}

#[test]
fn matches_cpython_same_operator_constant_loop_boolean_discard_locations() {
    for expression in [
        "left(item) and (True and False)",
        "left(item) or (False or True)",
    ] {
        let source =
            format!("def f(items):\n    for item in items:\n        {expression}\n    done()\n");
        common::assert_matches_cpython_marshal(
            &source,
            "protected_loop_condition_exit_locations.py",
        );
    }
}

#[test]
fn protected_nested_if_preserves_forward_exit_before_returning_handler() {
    let source = r"def probe(groups, manager, build, accept, stop):
    for items in groups:
        with manager():
            for item in items:
                try:
                    value = build(item)
                    if accept(value):
                        value = build(value)
                        if stop(value):
                            return value
                except KeyboardInterrupt:
                    return None
    return None
";
    common::assert_matches_cpython_marshal(source, "protected_nested_if_return_handler.py");
}

#[test]
fn protected_nested_if_preserves_forward_exit_before_continuing_handler() {
    let source = r"def probe(items, outer, inner, work, report):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    work(item)
        except Exception:
            report(item)
            continue
";
    common::assert_matches_cpython_marshal(source, "protected_nested_if_continue_handler.py");
}

#[test]
fn protected_nested_if_preserves_forward_exit_before_conditional_handler() {
    let source = r"def probe(items, outer, inner, work, recover, report):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    work(item)
        except Exception:
            if recover(item):
                report(item)
";
    common::assert_matches_cpython_marshal(source, "protected_nested_if_conditional_handler.py");
}

#[test]
fn matches_cpython_protected_backedges_after_nested_while_and_assertion() {
    for (filename, source) in [
        (
            "protected_try_inner_while_backedge_target.py",
            r"def visit(rows, choose, make_iterator, consume):
    try:
        for row in rows:
            try:
                fast = choose(row)
                if not fast:
                    iterator = make_iterator(row)
                    while iterator.has_next():
                        value = iterator.next()
                        try:
                            consume(value)
                        except Exception:
                            continue
            except Exception:
                continue
    except RuntimeError:
        raise
",
        ),
        (
            "protected_try_final_assert_backedge_target.py",
            r"def validate(pairs, report):
    for left, right in pairs:
        try:
            if isinstance(left, int):
                assert left == right
            else:
                assert (left == right).all()
        except Exception as error:
            report(error)
            raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_original_ordinary_try_compound_loop_exits() {
    for (filename, source) in [
        (
            "ordinary_try_compound_while_loop_exit_target.py",
            r"def run(groups, ready, remaining, consume):
    for group in groups:
        try:
            iterator = iter(group)
            while ready(iterator) and remaining():
                consume(next(iterator))
        except Exception:
            pass
",
        ),
        (
            "ordinary_try_single_while_loop_exit_control.py",
            r"def run(groups, ready, consume):
    for group in groups:
        try:
            iterator = iter(group)
            while ready(iterator):
                consume(next(iterator))
        except Exception:
            pass
",
        ),
        (
            "ordinary_try_completed_recovery_compound_break_target.py",
            r"def run(items, limit, recover):
    for item in items:
        try:
            try:
                recover(item)
            except LookupError:
                pass
            if limit is not None and item >= limit:
                break
        except Exception:
            continue
    return None
",
        ),
        (
            "ordinary_try_compound_break_linear_prefix_control.py",
            r"def run(items, limit, recover):
    for item in items:
        try:
            recover(item)
            if limit is not None and item >= limit:
                break
        except Exception:
            continue
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_sync_completed_try_compound_exit() {
    for (filename, source) in [
        (
            "sync_protected_completed_try_three_false_edges.py",
            r#"def inspect_values(items, depth, emit):
    try:
        for value in items:
            try:
                try:
                    text = str(value)
                except Exception as error:
                    emit(error)
                if depth < 4 and hasattr(value, "name") and any(part in value.name for part in ("red", "blue")):
                    emit(value)
            except Exception as error:
                emit(error)
    except Exception as error:
        emit(error)
"#,
        ),
        (
            "sync_protected_completed_try_single_false_edge_control.py",
            r"def inspect_values(items, depth, emit):
    try:
        for value in items:
            try:
                try:
                    text = str(value)
                except Exception as error:
                    emit(error)
                if depth < 4:
                    emit(value)
            except Exception as error:
                emit(error)
    except Exception as error:
        emit(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_async_completed_try_compound_exit() {
    for (filename, source) in [
        (
            "protected_loop_completed_try_and_exit.py",
            r"async def run(items, choose, consume):
    for item in items:
        try:
            if choose(item):
                if item.ready:
                    consume(item)
                else:
                    try:
                        value = obtain(item)
                    except Exception:
                        value = None
                    if isinstance(value, str) and value:
                        consume(value)
                        try:
                            await perform(value)
                            if item.done:
                                consume(item)
                        except Exception as error:
                            consume(error)
            elif item.other:
                consume(item)
            else:
                consume(None)
        except Exception as error:
            consume(error)
",
        ),
        (
            "protected_loop_completed_try_following_work_control.py",
            r"async def run(items, consume):
    for item in items:
        try:
            try:
                value = obtain(item)
            except Exception:
                value = None
            if isinstance(value, str) and value:
                await perform(value)
            consume(item)
        except Exception as error:
            consume(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_cleanup_break_owner_boundary() {
    let source = r"def run(stop, check, prepare, work, notify, recover, finish):
    try:
        while True:
            try:
                try:
                    prepare()
                except ValueError:
                    recover()
                work()
                if stop and check():
                    try:
                        notify()
                        break
                    finally:
                        recover()
            except Exception:
                recover()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_try_and_cleanup_break_boundary_control.py",
    );
}
