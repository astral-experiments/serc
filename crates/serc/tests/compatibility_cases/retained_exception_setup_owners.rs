use crate::common;

#[test]
fn matches_cpython_retained_setup_uniform_suffix_owners() {
    for tail in ["    return rows\n", "    yield obj\n"] {
        for (target, prefix, condition) in [
            ("line", "", "obj"),
            ("line", "            scratch = 1 + 2\n", "obj"),
            ("line", "", "predicate(obj)"),
            ("line", "            scratch = 1 + 2 + 3 + 4\n", "test(obj)"),
            ("(line, other)", "", "obj"),
            (
                "(line, other)",
                "            scratch = 1 + 2 + 3 + 4\n",
                "obj",
            ),
        ] {
            let source = format!(
                "def process(items, rows, obj, test):\n    try:\n        for {target} in items:\n{prefix}            try:\n                if {condition}:\n                    rows.append(obj)\n            except Error:\n                continue\n    except Error:\n        recover()\n{tail}",
            );
            common::assert_matches_cpython_marshal(&source, "retained_setup_uniform_suffix.py");
        }
    }
}

#[test]
fn matches_cpython_with_cleanup_after_one_terminal_conditional_arm() {
    for (definition, prefix) in [
        ("def", ""),
        ("async def", "    await prepare()\n"),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n    await prepare()\n"),
    ] {
        for managers in ["manager", "manager, second"] {
            for (first, second) in [
                ("work()", "return"),
                ("return", "value = work()"),
                ("value = work()", "return"),
                ("work()", "cleanup()"),
                ("return", "return"),
            ] {
                let source = format!(
                    "{definition} f(manager, second, flag):\n{prefix}    with {managers}:\n        if flag:\n            {first}\n        else:\n            {second}\n    after()\n"
                );
                common::assert_matches_cpython_marshal(&source, "with_conditional_exit_nop.py");
            }
        }
    }
}

#[test]
fn matches_cpython_finally_join_after_folded_infinite_loop() {
    for (definition, suspension, generator_prefix) in [
        ("def", "", ""),
        ("async def", "await ", ""),
        ("def", "", "    yield None\n"),
        ("async def", "await ", "    yield None\n"),
    ] {
        for (condition, loop_suffix) in [
            ("True", ""),
            ("True", "            if flag:\n                break\n"),
            ("flag", ""),
        ] {
            for handler in [
                "    except Error:\n        report()\n",
                "    except Error as error:\n        report(error)\n",
                "    except:\n        report()\n",
            ] {
                let source = format!(
                    "{definition} f(flag):\n{generator_prefix}    try:\n        while {condition}:\n            {suspension}work()\n{loop_suffix}{handler}    finally:\n        {suspension}cleanup()\n"
                );
                common::assert_matches_cpython_marshal(&source, "folded_loop_finally_join.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_empty_finally_body_pop_block_owners() {
    for (definition, suspension, suffix) in [
        ("def", "", ""),
        ("def", "", "    yield value\n"),
        ("async def", "", ""),
        ("async def", "await ", ""),
    ] {
        let source = format!(
            "{definition} f(value):\n    try:\n        pass\n    finally:\n        {suspension}work()\n{suffix}"
        );
        common::assert_matches_cpython_marshal(&source, "empty_finally_body_pop_block.py");
    }
}

#[test]
fn matches_cpython_marshal_for_generator_normal_finally_pop_block_owners() {
    for handler in [
        "raise",
        "status = 1\n        raise",
        "work()\n        raise",
    ] {
        for suffix in ["", "    work()\n"] {
            let source = format!(
                "def f(value):\n    status = 0\n    try:\n        yield value\n    except Exception:\n        {handler}\n    finally:\n        cleanup(status)\n{suffix}"
            );
            common::assert_matches_cpython_marshal(&source, "generator_finally_pop_block.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_retained_setups_in_copied_finally_bodies() {
    for prefix in ["def", "async def"] {
        for value in ["value", "None"] {
            for conditional in [false, true] {
                let body = if conditional {
                    "            if flag:\n                try:\n                    del value\n                except Exception:\n                    pass\n"
                } else {
                    "            try:\n                del value\n            except Exception:\n                pass\n"
                };
                let source = format!(
                    "{prefix} f(value, flag):\n    try:\n        try:\n            return {value}\n        finally:\n{body}    finally:\n        cleanup()\n"
                );
                common::assert_matches_cpython_marshal(&source, "copied_finally_setup_owners.py");
            }
        }
    }
}

#[test]
fn matches_cpython_retained_setup_after_removed_finally_pop_block() {
    for body in [
        "        work()\n",
        "        module.replace(tmp, path)\n",
        "        with manager() as handle:\n            handle.write()\n        module.replace(tmp, path)\n",
    ] {
        for condition in ["tmp", "tmp.exists()"] {
            for following in ["", "    after()\n"] {
                let source = format!(
                    "import module\ndef f(tmp, path):\n    try:\n{body}    finally:\n        try:\n            if {condition}:\n                tmp.unlink()\n        except Error:\n            pass\n{following}"
                );
                common::assert_matches_cpython_marshal(&source, "finally_pop_block_owner.py");
            }
        }
    }
}

#[test]
fn retained_setup_after_a_conditional_finally_entry() {
    for guard in ["not flag", "item.value"] {
        let source = format!(
            "def f(manager, item, flag):\n    with manager:\n        try:\n            assert flag\n        finally:\n            try:\n                if {guard}:\n                    close(item)\n            finally:\n                finish()\n"
        );
        common::assert_matches_cpython_marshal(&source, "conditional_finally_setup.py");
    }
}
