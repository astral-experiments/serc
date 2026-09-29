use crate::common;

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
