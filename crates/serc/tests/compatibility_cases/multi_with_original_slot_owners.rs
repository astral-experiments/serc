use crate::common;

#[test]
fn retained_setup_value_slots_preserve_function_entry_provenance() {
    for count in [4, 5] {
        let managers = vec!["manager()"; count].join(", ");
        for prefix in ["", "    prepare()\n", "    if flag:\n        prepare()\n"] {
            for expression in [
                "value or fallback",
                "value and fallback",
                "value.attr or fallback",
            ] {
                let source = format!(
                    "def f(value, fallback, flag):\n{prefix}    with {managers}:\n        try:\n            result = {expression}\n            return result\n        finally:\n            for child in children():\n                child.finish()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "retained_setup_value_function_entry.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_multi_with_original_slot_owners() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for count in [3, 4, 5, 6, 12] {
            let managers = "manager, ".repeat(count);
            let managers = managers.trim_end_matches(", ");
            let source = format!(
                "{definition} f(flag):\n    with {managers}:\n        if flag:\n            work()\n{tail}"
            );
            common::assert_matches_cpython_marshal(&source, "multi_with_original_slots.py");
        }
    }
}

#[test]
fn matches_cpython_multi_with_condition_slot_widths() {
    for definition in ["def", "async def"] {
        for condition in [
            "flag",
            "not flag",
            "flag is None",
            "flag is not None",
            "flag < value",
            "flag in value",
            "local()",
            "flag and local()",
        ] {
            let managers = "factory(value=flag), ".repeat(5);
            let managers = managers.trim_end_matches(", ");
            let source = format!(
                "{definition} f(flag, value, local):\n    with {managers}:\n        if {condition}:\n            value = flag\n        finish(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "multi_with_condition_widths.py");
        }
    }
}

#[test]
fn matches_cpython_multi_with_prologue_and_outer_manager_slots() {
    for definition in ["def", "async def"] {
        for count in [4, 6] {
            let managers = "manager(), ".repeat(count);
            let managers = managers.trim_end_matches(", ");
            for source in [
                format!(
                    "def outer(flag, value, manager):\n    {definition} f():\n        with {managers}:\n            if flag:\n                value = flag\n            finish(value)\n    return f\n"
                ),
                format!(
                    "{definition} f(flag, value, manager):\n    def captured():\n        return flag\n    with {managers}:\n        if flag:\n            value = flag\n        finish(value)\n"
                ),
                format!(
                    "{definition} f(flag, value, manager):\n    with outer_manager:\n        with {managers}:\n            if flag:\n                value = flag\n            finish(value)\n"
                ),
            ] {
                common::assert_matches_cpython_marshal(&source, "multi_with_prologue_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_multi_with_uncertified_boundary_controls() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for count in [3, 4] {
            let managers = "manager(), ".repeat(count);
            let managers = managers.trim_end_matches(", ");
            let source = format!(
                "{definition} f(flag, value, manager):\n    with {managers}:\n        pass\n        if flag:\n            value = flag\n        finish(value)\n{tail}"
            );
            common::assert_matches_cpython_marshal(&source, "multi_with_retained_nop_control.py");
        }
        for count in [1, 3] {
            let managers = "factory(value=flag), ".repeat(count);
            let managers = managers.trim_end_matches(", ");
            let source = format!(
                "{definition} f(flag, value):\n    try:\n        with {managers}:\n            if flag:\n                value = flag\n            finish(value)\n    except Error:\n        recover()\n{tail}"
            );
            common::assert_matches_cpython_marshal(&source, "multi_with_foreign_setup_control.py");
        }
    }
}

#[test]
fn matches_cpython_multi_with_jump_target_setup_controls() {
    for definition in ["def", "async def"] {
        for count in [4, 6] {
            for expression in [
                "(manager() if value else factory())",
                "(manager() or factory())",
            ] {
                let managers = format!("{expression}, ").repeat(count);
                let managers = managers.trim_end_matches(", ");
                let source = format!(
                    "{definition} f(flag, value, manager):\n    with {managers}:\n        if flag:\n            value = flag\n        finish(value)\n"
                );
                common::assert_matches_cpython_marshal(&source, "multi_with_setup_join_control.py");
            }
        }
    }
}
