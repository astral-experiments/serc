use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_copied_finally_markers_after_unnamed_handler_returns() {
    for kind in ["coroutine", "generator", "async_generator"] {
        for value in ["", " None"] {
            if kind == "async_generator" && !value.is_empty() {
                continue;
            }
            for condition in ["value is None", "value is not None"] {
                let source = format!(
                    "{}def f(value, manager):\n    try:\n        work()\n    except Error:\n        return{value}\n    finally:\n        if {condition}:\n            cleanup(value)\n{}",
                    if kind == "generator" { "" } else { "async " },
                    if kind == "coroutine" {
                        ""
                    } else {
                        "    yield value\n"
                    },
                );
                assert_matches_cpython_marshal(&source, "handler_return_none_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_copied_finally_markers_after_handler_call_and_return() {
    for kind in ["coroutine", "generator", "async_generator"] {
        let source = format!(
            "{}def f(value, manager):\n    try:\n        work()\n    except Error:\n        work()\n        return\n    finally:\n        if value:\n            cleanup(value)\n{}",
            if kind == "generator" { "" } else { "async " },
            if kind == "coroutine" {
                ""
            } else {
                "    yield value\n"
            },
        );
        assert_matches_cpython_marshal(&source, "handler_call_return_finally.py");
    }
}

#[test]
fn matches_cpython_handler_cleanup_slots_with_preserved_values_and_names() {
    for asynchronous in [false, true] {
        for handler_name in ["", " as error"] {
            let source = format!(
                "{}def f(value, manager):\n    try:\n        work()\n    except Error{handler_name}:\n        return value\n    finally:\n        if value is None:\n            cleanup(value)\n",
                if asynchronous { "async " } else { "" },
            );
            assert_matches_cpython_marshal(&source, "handler_preserved_value_finally.py");
        }
    }
}

#[test]
fn matches_cpython_handler_return_slots_after_value_and_branch_boundaries() {
    for handler in [
        "        return (\n            value\n        )\n",
        "        if flag:\n            work()\n        return\n",
    ] {
        let source = format!(
            "async def f(value, flag, manager):\n    try:\n        work()\n    except Error:\n{handler}    finally:\n        if value is None:\n            cleanup(value)\n",
        );
        assert_matches_cpython_marshal(&source, "handler_value_branch_boundary.py");
    }
}

#[test]
fn matches_cpython_named_handler_constant_return_cleanup_slots() {
    for binding in ["", " as exc"] {
        for returned in ["", " None", " 17", " value"] {
            for condition in ["caught is None", "caught is not None"] {
                let source = format!(
                    "def f(flag, value):\n    caught = None\n    try:\n        return get()\n    except Error{binding}:\n        caught = {}\n        if flag:\n            return{returned}\n    finally:\n        if {condition}:\n            cleanup(caught)\n            caught = None\n",
                    if binding.is_empty() { "error" } else { "exc" },
                );
                assert_matches_cpython_marshal(&source, "named_handler_constant_return_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_inner_handler_return_before_outer_except_and_finally() {
    for nested_while in [false, true] {
        let loop_header = if nested_while {
            "        while flag:\n"
        } else {
            ""
        };
        let indent = if nested_while {
            "            "
        } else {
            "        "
        };
        for binding in ["", " as error"] {
            for returned in ["", " value"] {
                let source = format!(
                    "def f(flag, value):\n    try:\n{loop_header}{indent}try:\n{indent}    work()\n{indent}except Inner{binding}:\n{indent}    return{returned}\n    except Outer:\n        recover()\n    finally:\n        cleanup()\n    finished()\n",
                );
                assert_matches_cpython_marshal(&source, "inner_handler_return_outer_try.py");
            }
        }
    }
}

#[test]
fn matches_cpython_inner_handler_return_preserves_copied_finally_marker_owners() {
    for kind in ["coroutine", "generator", "async_generator"] {
        let asynchronous = kind != "generator";
        let source = format!(
            "{}def f(flag, value):\n    try:\n        try:\n            {}work()\n        except Inner:\n            return\n    except Outer:\n        recover()\n    finally:\n        if flag is not None:\n            cleanup()\n{}",
            if asynchronous { "async " } else { "" },
            if asynchronous { "await " } else { "" },
            if kind == "coroutine" {
                ""
            } else {
                "    yield value\n"
            },
        );
        assert_matches_cpython_marshal(&source, "inner_handler_copied_finally_marker_owners.py");
    }
}

#[test]
fn matches_cpython_outer_try_owner_on_named_handler_constant_finally() {
    for returned in [
        "", "False", "True", "None", "3", "'value'", "b'value'", "1.25",
    ] {
        for cleanup in ["finish()", "if path.exists():\n    path.remove()"] {
            let source = format!(
                "def f(path):\n    try:\n        try:\n            work()\n        except ValueError as error:\n            report(error)\n            return {returned}\n        finally:\n{}        finished()\n    except RuntimeError:\n        recover()\n",
                crate::common::indent(&crate::common::indent(&crate::common::indent(cleanup)))
            );
            assert_matches_cpython_marshal(&source, "named_handler_constant_finally_owner.py");
        }
    }
}

#[test]
fn matches_cpython_nested_finalizer_method_cleanup_donors() {
    let source = r#"def f(config, previous, env):
    try:
        if env:
            prepare()
        try:
            value = config.read()
            try:
                return convert(value)
            except ValueError:
                return value
        finally:
            config.close()
    except UsageError as error:
        report(error)
        return status.usage
    finally:
        if previous is None:
            env.pop("KEY", None)
        else:
            env["KEY"] = previous
"#;
    for (filename, source) in [
        ("copied_method_cleanup_original_entry.py", source.to_owned()),
        ("copied_method_cleanup_named_handler.py", source.replace("except ValueError:", "except ValueError as error:")),
        ("copied_method_cleanup_one_ordinary_try.py", source.replace("    except UsageError as error:\n        report(error)\n        return status.usage\n", "")),
        ("copied_method_cleanup_nested_condition.py", source.replace("            env.pop(\"KEY\", None)", "            if env:\n                env.pop(\"KEY\", None)")),
    ] {
        assert_matches_cpython_marshal(&source, filename);
    }
}
