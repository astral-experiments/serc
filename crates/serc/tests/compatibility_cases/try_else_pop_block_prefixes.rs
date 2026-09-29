use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_try_else_pop_block_prefixes() {
    for condition in [
        "value",
        "value < other",
        "value or other",
        "not (value and other)",
        "False or value",
        "True and value",
    ] {
        for action in ["return 1", "raise Error"] {
            for otherwise in ["consume(value)", "pass", "return 2"] {
                for prefix in ["", "async "] {
                    let source = format!(
                        "{prefix}def f(value, other):\n    try:\n        if {condition}:\n            {action}\n    except Error:\n        recover()\n    else:\n        {otherwise}\n    after()\n"
                    );
                    assert_matches_cpython_marshal(&source, "try_else_pop_block_prefix.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_try_else_pop_block_prefixes_inside_loops() {
    for header in ["for item in items:", "while value:"] {
        for condition in ["value", "value < other", "value or other"] {
            for action in ["break", "continue"] {
                let source = format!(
                    "def f(value, other, items):\n    {header}\n        try:\n            if {condition}:\n                {action}\n        except Error:\n            recover()\n        else:\n            consume(value)\n    after()\n"
                );
                assert_matches_cpython_marshal(&source, "try_else_loop_pop_block_prefix.py");
            }
        }
    }
}

#[test]
fn distinguishes_original_loop_exits_before_try_else() {
    for body in [
        "while True:\n    work()",
        "while 1:\n    pass",
        "try:\n    work()\nfinally:\n    while True:\n        work()",
        "for item in items:\n    work(item)\nelse:\n    while True:\n        work()",
        "for item in items:\n    work(item)\nelse:\n    return 1",
    ] {
        for condition in ["value", "value or other"] {
            let mut indented = String::new();
            for line in body.lines() {
                indented.push_str("            ");
                indented.push_str(line);
                indented.push('\n');
            }
            let source = format!(
                "def f(value, other, items):\n    try:\n        if {condition}:\n{indented}    except Error:\n        recover()\n    else:\n        consume(value)\n    after()\n"
            );
            assert_matches_cpython_marshal(&source, "try_else_original_loop_exit.py");
        }
    }
}

#[test]
fn retains_normal_try_pop_block_after_nested_handler_exit() {
    for (prefix, body) in [
        ("", "work(value)"),
        ("async ", "await work(value)"),
        ("", "yield value"),
        ("async ", "yield value"),
    ] {
        let source = format!(
            "{prefix}def f(value):\n    try:\n        try:\n            {body}\n        except InnerError:\n            raise\n    except OuterError:\n        raise\n    else:\n        record(value)\n    after()\n"
        );
        assert_matches_cpython_marshal(&source, "nested_handler_normal_pop_block.py");
    }
}

#[test]
fn preserves_copyable_else_exits_after_folded_finally_loops() {
    for (otherwise, following) in [
        ("return value", "after()"),
        ("return 1", "after()"),
        ("raise Error", "after()"),
        ("del value", "raise"),
    ] {
        let source = format!(
            "def f(value, other, items):\n    try:\n        if value:\n            try:\n                work()\n            finally:\n                while True:\n                    work()\n    except Error:\n        recover()\n    else:\n        {otherwise}\n    {following}\n"
        );
        assert_matches_cpython_marshal(&source, "copyable_else_after_finally_loop.py");
    }
}

#[test]
fn distinguishes_folded_loop_breaks_at_normal_try_exits() {
    for (prefix, read, otherwise) in [
        (
            "async ",
            "await read()",
            "done = False\n        await finish()",
        ),
        ("", "read()", "done = False\n        finish()"),
        ("", "read()", "return value"),
        ("", "read()", "return 1"),
    ] {
        let source = format!(
            "{prefix}def f():\n    try:\n        while True:\n            value = {read}\n            if not value:\n                break\n            consume(value)\n    except Error:\n        recover()\n    else:\n        {otherwise}\n    after()\n"
        );
        assert_matches_cpython_marshal(&source, "folded_loop_break_normal_exit.py");
    }
}

fn assert_loop_break_before_else_condition(case: usize) {
    let cases = [
        (
            "async ",
            "await read()",
            "True",
            "",
            "record(value)\n        if value:\n            consume(value)\n        else:\n            missing()",
        ),
        (
            "",
            "read()",
            "True",
            "",
            "record(value)\n        if value:\n            consume(value)\n        else:\n            missing()",
        ),
        (
            "",
            "read()",
            "1",
            "",
            "record(value)\n        if value:\n            consume(value)\n        else:\n            missing()",
        ),
        (
            "",
            "read()",
            "running",
            "",
            "record(value)\n        if value:\n            consume(value)\n        else:\n            missing()",
        ),
        (
            "",
            "read()",
            "True",
            "        else:\n            exhausted()\n",
            "record(value)\n        if value:\n            consume(value)\n        else:\n            missing()",
        ),
        ("", "read()", "True", "", "return value"),
        ("", "read()", "True", "", "return 1"),
        (
            "",
            "read()",
            "True",
            "",
            "if value:\n            consume(value)",
        ),
        (
            "",
            "read()",
            "True",
            "        else:\n            return value\n",
            "if value:\n            consume(value)",
        ),
        (
            "",
            "read()",
            "True",
            "        else:\n            raise OuterError()\n",
            "if value:\n            consume(value)",
        ),
    ];
    let (prefix, read, condition, loop_else, otherwise) = cases[case];
    let source = format!(
        "{prefix}def f(running):\n    try:\n        while {condition}:\n            try:\n                value = {read}\n                break\n            except InnerError as exc:\n                if retry(exc):\n                    recover()\n                else:\n                    raise\n{loop_else}    except OuterError:\n        failed()\n    else:\n        {otherwise}\n    after()\n"
    );
    assert_matches_cpython_marshal(&source, "folded_loop_break_else_condition.py");
}

#[test]
fn preserves_async_folded_loop_break_before_else_condition() {
    assert_loop_break_before_else_condition(0);
}

#[test]
fn preserves_folded_loop_break_before_else_condition() {
    assert_loop_break_before_else_condition(1);
}

#[test]
fn preserves_integer_folded_loop_break_before_else_condition() {
    assert_loop_break_before_else_condition(2);
}

#[test]
fn preserves_dynamic_loop_break_before_else_condition() {
    assert_loop_break_before_else_condition(3);
}

#[test]
fn preserves_folded_loop_else_break_before_else_condition() {
    assert_loop_break_before_else_condition(4);
}

#[test]
fn preserves_folded_loop_break_before_else_name_return() {
    assert_loop_break_before_else_condition(5);
}

#[test]
fn preserves_folded_loop_break_before_else_constant_return() {
    assert_loop_break_before_else_condition(6);
}

#[test]
fn preserves_folded_loop_break_before_short_else_condition() {
    assert_loop_break_before_else_condition(7);
}

#[test]
fn preserves_folded_loop_returning_else_before_short_else_condition() {
    assert_loop_break_before_else_condition(8);
}

#[test]
fn preserves_folded_loop_raising_else_before_short_else_condition() {
    assert_loop_break_before_else_condition(9);
}

#[test]
fn preserves_original_linear_try_pop_slot_after_compaction() {
    for (filename, expression) in [
        ("try_else_erased_pop_prefix_zero_control.py", "value"),
        (
            "try_else_erased_pop_prefix_two_control.py",
            "first(second(value))",
        ),
        (
            "try_else_erased_pop_prefix.py",
            "first(second(third(fourth(value))))",
        ),
    ] {
        let source = format!(
            "def f(value, enabled):\n    try:\n        result = {expression}\n    except Error:\n        return None\n    else:\n        if enabled:\n            record(result)\n    return result\n"
        );
        assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn excludes_retained_conditional_pop_blocks_from_suspension_handlers() {
    for (prefix, action) in [
        ("", "work()"),
        ("async ", "await work()"),
        ("", "yield work()"),
        ("async ", "yield work()"),
    ] {
        let source = format!(
            "{prefix}def f(condition, work):\n    try:\n        if condition:\n            {action}\n        else:\n            {action}\n    except Error:\n        pass\n    else:\n        return\n"
        );
        assert_matches_cpython_marshal(&source, "conditional_pop_block_suspension_owner.py");
    }
}

#[test]
fn excludes_retained_conditional_pop_blocks_from_enclosing_tries() {
    for (prefix, action) in [
        ("", "work()"),
        ("async ", "await work()"),
        ("", "yield work()"),
        ("async ", "yield work()"),
    ] {
        let source = format!(
            "{prefix}def f(condition, work):\n    try:\n        try:\n            if condition:\n                {action}\n            else:\n                {action}\n        except Error:\n            pass\n        else:\n            raise Failed\n    except Other:\n        recover()\n"
        );
        assert_matches_cpython_marshal(&source, "conditional_pop_block_enclosing_owner.py");
    }
}
