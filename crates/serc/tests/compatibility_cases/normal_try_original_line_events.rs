use crate::common;

#[test]
fn preserves_outer_condition_before_multiple_shared_return_paths() {
    for (first, second) in [
        ("value = first", "value = second"),
        ("first(value)", "second(value)"),
    ] {
        let source = format!(
            "def select(value, active, choose, first, second):\n    try:\n        if active:\n            if choose == 0:\n                {first}\n            elif choose == 1:\n                {second}\n            else:\n                raise ValueError()\n    except ValueError:\n        raise\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "outer_condition_shared_return.py");
    }
}

#[test]
fn matches_cpython_shared_inner_try_exit_line_events() {
    for (name, tail) in [
        ("call", "if inner:\n    work(value)"),
        ("assignment", "if inner:\n    value = inner"),
        (
            "nested_conditions",
            "if inner:\n    if value:\n        work(value)",
        ),
    ] {
        let source = format!(
            "def f(outer, inner, value, work):\n    try:\n        if outer:\n            try:\n                {}\n            except ValueError:\n                pass\n    except Exception:\n        work(None)\n    work(value)\n    return value\n",
            tail.replace('\n', "\n                "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("shared_inner_try_exit_{name}.py"),
        );
    }
}

#[test]
fn compacts_assertion_pop_before_matching_normal_exit() {
    for (header, condition, name) in [
        ("with manager():", "isinstance(value, Type)", " as error"),
        ("for item in items:", "5 >= value >= 1", ""),
    ] {
        let source = format!(
            "def f(flag, other, items):\n    {header}\n        if flag:\n            try:\n                value = load()\n                assert {condition}\n            except Error{name}:\n                raise BadValue(value)\n        else:\n            value = other\n        consume(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "sync_assertion_pop_compaction.py");
    }
}

#[test]
fn preserves_borrows_after_guarded_nested_handler_cleanup() {
    for scope in ["def", "async def"] {
        for guard in ["x", "not x", "x is None", "x == y"] {
            for inner_name in ["", " as inner"] {
                for (wrapper, indent) in [
                    ("", "    "),
                    ("    with manager():\n", "        "),
                    ("    for item in x:\n", "        "),
                ] {
                    let source = format!(
                        "{scope} f(x, y):\n{wrapper}{indent}try:\n{indent}    work(x)\n{indent}except E as error:\n{indent}    if {guard}:\n{indent}        try:\n{indent}            work(y)\n{indent}        except E{inner_name}:\n{indent}            work(x)\n{indent}return use(x)\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "guarded_nested_handler_borrows.py",
                    );
                }
            }
        }
    }
}
