use crate::common;

#[test]
fn nested_for_else_pass_backedge_location() {
    common::assert_matches_cpython_marshal(
        r"def f(items, attempts, ready):
    for attempt in attempts:
        for item in items:
            work(item)
            if ready():
                break
        else:
            prepare()
            if not ready():
                return attempt
            else:
                pass
    return None
",
        "nested_for_else_pass_backedge_location.py",
    );
}

#[test]
fn preserves_nested_for_else_backedge_donors() {
    for (name, tail) in [
        ("conditional_pass", "if ready():\n    pass"),
        ("conditional_return", "if not ready():\n    return attempt"),
        (
            "located_else",
            "if not ready():\n    return attempt\nelse:\n    finish()",
        ),
    ] {
        let source = format!(
            "def f(items, attempts, ready):\n    for attempt in attempts:\n        for item in items:\n            work(item)\n            if ready():\n                break\n        else:\n            prepare()\n            {}\n    return None\n",
            tail.replace('\n', "\n            "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("nested_for_else_backedge_{name}.py"),
        );
    }
}

#[test]
fn match_or_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case True | False:
                pass
            case _:
                raise ValueError(item)
",
        "match_or_pass.py",
    );
}

#[test]
fn match_literal_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case 1:
                pass
            case _:
                raise ValueError(item)
",
        "match_literal_pass.py",
    );
}

#[test]
fn for_simple_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        pass
",
        "for_simple_pass.py",
    );
}

#[test]
fn match_or_work() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case True | False:
                work(item)
            case _:
                raise ValueError(item)
",
        "match_or_work.py",
    );
}

#[test]
fn for_body_call() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        work(item)
",
        "for_body_call.py",
    );
}

#[test]
fn for_runtime_if() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        if ready(item):
            work(item)
        else:
            finish(item)
",
        "for_runtime_if.py",
    );
}

#[test]
fn copied_match_condition_backedges_keep_their_original_order() {
    for (filename, source) in [
        (
            "match_and_assignment_backedges.py",
            r"def f(values, left, right):
    result = None
    for value in values:
        match value:
            case 1:
                if left and right:
                    result = value
                    left = False
                    right = True
    return result
",
        ),
        (
            "match_and_nested_return_backedges.py",
            r"def f(values, left, right, accept):
    for value in values:
        match value:
            case 0:
                return False
            case 1:
                if left and right:
                    if accept(value):
                        return True
    return False
",
        ),
        (
            "nested_match_or_and_yield_backedges.py",
            r"def f(groups, left, right):
    for group in groups:
        for value in group:
            match value:
                case [*items] | (*items,):
                    if any(left(item) for item in items) and not any(
                        right(item) for item in items
                    ):
                        yield value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn shared_original_for_return_backedges_keep_threading() {
    common::assert_matches_cpython_marshal(
        r"def f(items, attempts, first, second):
    for attempt in attempts:
        for item in items:
            work(item)
            if first():
                break
        else:
            prepare()
            if first() and second():
                return attempt
    return None
",
        "compound_conditional_return.py",
    );
}

#[test]
fn final_value_match_pass_threads_to_loop_backedge() {
    for (name, prefix) in [("sync", "def"), ("async", "async def")] {
        let source = format!(
            "{prefix} f(items):\n    for item in items:\n        match item:\n            case 1:\n                use(item)\n            case 2:\n                pass\n            case _:\n                fallback(item)\n    return None\n",
        );
        common::assert_matches_cpython_marshal(&source, &format!("match_pass_{name}.py"));
    }
}

#[test]
fn match_pass_backedge_preserves_other_case_exits() {
    for (name, arms, after) in [
        (
            "nonterminal",
            "case 2:\n    pass\ncase _:\n    fallback(item)",
            "        after(item)\n",
        ),
        ("no_default", "case 2:\n    pass", ""),
        (
            "copied_subject",
            "case 1:\n    pass\ncase 2:\n    use(item)\ncase _:\n    fallback(item)",
            "",
        ),
        (
            "nonempty",
            "case 2:\n    use(item)\ncase _:\n    fallback(item)",
            "",
        ),
    ] {
        let arms = arms
            .lines()
            .flat_map(|line| ["            ", line, "\n"])
            .collect::<String>();
        let source = format!(
            "def f(items):\n    for item in items:\n        match item:\n{arms}{after}    return None\n"
        );
        common::assert_matches_cpython_marshal(&source, &format!("match_pass_control_{name}.py"));
    }
}

#[test]
fn retained_try_condition_after_inlined_comprehensions() {
    for prefix in [
        "counts = {key: factory() for key in keys}\n    other = {key: factory() for key in keys}\n    output = None",
        "counts = {}\n    other = {}\n    output = None",
    ] {
        let source = format!(
            "def f(keys, dry_run):\n    {prefix}\n    try:\n        if not dry_run:\n            prepare()\n        work(counts, other)\n    finally:\n        if output is not None:\n            output.close()\n",
        );
        common::assert_matches_cpython_marshal(&source, "retained_try_after_comprehensions.py");
    }
}

#[test]
fn async_retained_try_condition_after_straight_line_prefix() {
    common::assert_matches_cpython_marshal(
        r"async def f(flag, client):
    previous = None
    options = {'enabled': True}
    try:
        if flag:
            await client.update(options)
        else:
            await client.create(options)
    except Exception:
        await client.rollback(previous)
        raise
    return options
",
        "async_retained_try_after_prefix.py",
    );
}

#[test]
fn named_handler_return_finally_condition() {
    common::assert_matches_cpython_marshal(
        r"async def f(task, pending):
    try:
        return await task()
    except ValueError as exc:
        return result(error=exc)
    finally:
        if pending is not None:
            pending.cancel()
            await finish(pending)
",
        "named_handler_return_finally_condition.py",
    );
}

#[test]
fn named_handler_branching_return_finally_condition() {
    common::assert_matches_cpython_marshal(
        r"async def f(task, pending, progress):
    try:
        return await task()
    except ValueError as exc:
        return result(
            error=exc,
            link=None if progress is None else progress.link,
            response=None if progress is None else progress.response,
        )
    finally:
        if pending is not None:
            pending.cancel()
            await finish(pending)
",
        "named_handler_branching_return_finally_condition.py",
    );
}

#[test]
fn nested_try_except_return_preserves_finally_pop_slots() {
    for ordinary_depth in 1..=3 {
        let mut body =
            "value = await task()\nif flag:\n    return False, value\nreturn True, ''\n".to_owned();
        for _ in 0..ordinary_depth {
            body = format!(
                "try:\n    {}\nexcept ValueError:\n    raise\n",
                body.replace('\n', "\n    ").trim_end(),
            );
        }
        let source = format!(
            "async def f(task, session, flag):\n    try:\n        {}\n    finally:\n        if session is not None:\n            await session.close()\n",
            body.trim_end().replace('\n', "\n        "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("nested_try_except_return_finally_pop_slots_{ordinary_depth}.py"),
        );
    }
}

#[test]
fn async_generator_manager_finally_none() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, pending):
    async with manager:
        try:
            yield
        finally:
            if pending is not None:
                await pending.close()
",
        "async_generator_manager_finally_none.py",
    );
}

#[test]
fn coroutine_manager_finally_none() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, pending, task):
    async with manager:
        try:
            await task()
        except ValueError:
            raise
        finally:
            if pending is not None:
                await pending.close()
",
        "coroutine_manager_finally_none.py",
    );
}

#[test]
fn generator_manager_finally_none() {
    common::assert_matches_cpython_marshal(
        r"def f(manager, pending):
    with manager:
        try:
            yield
        finally:
            if pending is not None:
                pending.close()
",
        "generator_manager_finally_none.py",
    );
}

#[test]
fn coroutine_manager_finally_cell_none() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, pending, task):
    def current():
        return pending
    with manager:
        try:
            await task(current)
        finally:
            if pending is not None:
                pending.close()
",
        "coroutine_manager_finally_cell_none.py",
    );
}

#[test]
fn manager_finally_none_after_assertion_success() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, pending, task):
    async with manager:
        try:
            result = await task()
            assert result is not None
        finally:
            if pending is not None:
                await pending.close()
",
        "manager_finally_none_after_assertion_success.py",
    );
}

#[test]
fn manager_finally_none_after_nested_try_exit_copy() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, pending, task):
    async with manager:
        try:
            try:
                await task()
            except ValueError:
                report()
        finally:
            if pending is not None:
                pending.close()
",
        "manager_finally_none_after_nested_try_exit_copy.py",
    );
}

#[test]
fn conditional_manager_after_late_setups() {
    for managers in [2, 10, 12] {
        let prefix = (0..managers).map(|_| "manager(), ").collect::<String>();
        let source = format!(
            "def f(value):\n    with ({prefix}manager(value) if value is not None else manager()):\n        yield value\n"
        );
        common::assert_matches_cpython_marshal(&source, "conditional_manager_after_late_setups.py");
    }
}

#[test]
fn manager_entry_owner_after_local_pair_fusion() {
    for managers in [2, 10, 12] {
        let mut entries = vec!["manager((1, 2, 3, 4, 5))"; managers];
        entries[managers - 2] = "manager((left, right))";
        let source = format!(
            "async def f(left, right, flag):\n    with ({}):\n        if flag:\n            await work()\n",
            entries.join(", "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            "manager_entry_owner_after_local_pair_fusion.py",
        );
    }
}

#[test]
fn manager_entry_owner_without_local_pair_fusion() {
    for managers in [2, 10, 12] {
        let entries = vec!["manager((1, 2, 3, 4, 5))"; managers];
        let source = format!(
            "def f(flag):\n    with ({}):\n        assert predicate(flag) == 1\n",
            entries.join(", "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            "manager_entry_owner_without_local_pair_fusion.py",
        );
    }
}

#[test]
fn retained_setup_cell_after_resumable_prefix() {
    for asynchronous in [false, true] {
        let qualifier = if asynchronous { "async " } else { "" };
        let source = format!(
            "{qualifier}def f(stop, flag):\n    if stop:\n        return\n    values = (first(), second(), third(), fourth())\n    try:\n        if flag:\n            consume(values)\n    except ValueError:\n        recover()\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "retained_setup_cell_after_resumable_prefix.py",
        );
    }
}

#[test]
fn boolean_manager_expression_retains_source_owner() {
    for managers in [2, 10, 12] {
        let mut entries = vec!["manager((1, 2, 3, 4, 5))"; managers];
        entries.push("manager(refresh or replacement())");
        let source = format!(
            "def f(refresh):\n    with ({}):\n        consume()\n",
            entries.join(", "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            "boolean_manager_expression_retains_source_owner.py",
        );
    }
}

#[test]
fn retained_setup_in_original_finally_array() {
    for source in [
        "def f(flag):\n    try:\n        yield 1\n    except BaseException:\n        raise\n    finally:\n        try:\n            if flag:\n                cleanup()\n        except OSError:\n            recover()\n",
        "def f(flag):\n    with manager():\n        try:\n            work()\n        except BaseException:\n            raise\n        finally:\n            try:\n                if flag:\n                    cleanup()\n            except OSError:\n                recover()\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "retained_setup_in_original_finally_array.py",
        );
    }
}

#[test]
fn matches_cpython_folded_loop_backedge_owner_prefixes() {
    for (filename, source) in [
        (
            "protected_async_generator_folded_false_tail.py",
            r"async def run(wait):
    try:
        while True:
            await wait()
            if False:
                yield None
    except ValueError:
        raise
",
        ),
        (
            "protected_async_generator_runtime_tail_control.py",
            r"async def run(wait, ready):
    try:
        while True:
            await wait()
            if ready:
                yield None
    except ValueError:
        raise
",
        ),
        (
            "manager_iterator_assert_folded_true_arm.py",
            r"def run(manager, items, ready, valid):
    with manager:
        for item in items:
            assert valid(item) if ready(item) else 1.0
",
        ),
        (
            "manager_iterator_assert_runtime_arm_control.py",
            r"def run(manager, items, ready, valid, fallback):
    with manager:
        for item in items:
            assert valid(item) if ready(item) else fallback(item)
",
        ),
        (
            "finally_manager_iterator_folded_false_break.py",
            r"def run(manager, items, limit, ready, cleanup):
    try:
        with manager:
            for item in items:
                if ready(item) if limit is not None else False:
                    break
    finally:
        cleanup()
",
        ),
        (
            "finally_manager_iterator_runtime_break_control.py",
            r"def run(manager, items, limit, ready, cleanup, fallback):
    try:
        with manager:
            for item in items:
                if ready(item) if limit is not None else fallback(item):
                    break
    finally:
        cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn guarded_match_common_tail_keeps_condition_backedge_owner() {
    let source = r"def f(items, method_name, other_type):
    for entry in items:
        bindings = {}
        if callable := entry.methods.get(method_name):
            match callable.signature:
                case Signature((left, right)) if right.matches(bindings, other_type):
                    yield left.convert(bindings)
";
    common::assert_matches_cpython_marshal(source, "guarded_match_common_tail.py");
}

#[test]
fn nested_for_else_shared_tail_keeps_each_predecessor_location() {
    let source = r"def f(entries, patterns, classes):
    for name, obj in entries:
        if isfunction(obj):
            consume(name)
        elif isclass(obj):
            for pattern in patterns:
                if pattern.matches(name):
                    break
            else:
                attrs = attributes(obj)
                if attrs:
                    classes[name] = attrs
";
    common::assert_matches_cpython_marshal(source, "nested_for_else_shared_tail.py");
}
