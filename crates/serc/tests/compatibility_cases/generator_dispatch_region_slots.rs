use crate::common;

#[test]
fn matches_cpython_generator_dispatch_after_closure_prologue() {
    for definition in ["def", "async def"] {
        for callable in ["any", "all", "tuple"] {
            for handler in [
                "except Error:\n            pass",
                "except Error as error:\n            raise",
            ] {
                let source = format!(
                    "{definition} f(values):\n    def inner():\n        try:\n            value = work(values)\n            return {callable}(item for item in values)\n        {handler}\n    return inner\n",
                );
                common::assert_matches_cpython_marshal(&source, "closure_dispatch_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_dispatch_after_unpacked_assignment() {
    for definition in ["def", "async def"] {
        let await_prefix = if definition == "def" { "" } else { "await " };
        for callable in ["any", "all", "tuple"] {
            for handler in [
                "except Error:\n        pass",
                "except Error as error:\n        raise",
            ] {
                let source = format!(
                    "{definition} f(values, manager):\n    {await_prefix}before()\n    left, right = values\n    try:\n        result = {await_prefix}manager.work(values={callable}(item for item in values))\n    {handler}\n",
                );
                common::assert_matches_cpython_marshal(&source, "unpacked_dispatch_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_dispatch_after_normal_try_exit() {
    for definition in ["def", "async def"] {
        for callable in ["any", "all", "tuple"] {
            for body in ["value = None", "value = work(values)"] {
                let source = format!(
                    "{definition} f(values, flag):\n    if flag:\n        return None\n    try:\n        {body}\n    except Error:\n        pass\n    else:\n        result = {callable}(item for item in values)\n",
                );
                common::assert_matches_cpython_marshal(&source, "try_else_dispatch_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_dispatch_after_handler_join_in_with() {
    for definition in ["def", "async def"] {
        for callable in ["any", "all", "tuple"] {
            for handler in [
                "except Error:\n            pass",
                "except Error as error:\n            raise",
            ] {
                let source = format!(
                    "{definition} f(values, manager):\n    with manager:\n        try:\n            value = work(values)\n        {handler}\n        return {callable}(item for item in values)\n",
                );
                common::assert_matches_cpython_marshal(&source, "with_handler_dispatch_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_generator_dispatch_at_original_while_entries() {
    for definition in ["def", "async def"] {
        let suspension = if definition == "async def" {
            "    await prepare()\n"
        } else {
            ""
        };
        for callable in ["any", "all"] {
            for prefix in ["prepare()", "choices = {'one', 'two', 'three'}"] {
                for loop_body in [
                    format!(
                        "        while {callable}(value for value in values):\n            work()\n"
                    ),
                    format!(
                        "        while True:\n            result = {callable}(value for value in values)\n            if ready:\n                break\n"
                    ),
                ] {
                    let source = format!(
                        "{definition} f(values, ready):\n{suspension}    {prefix}\n    try:\n{loop_body}    finally:\n        cleanup()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "while_dispatch_entries.py");
                }
            }
        }
    }
}
