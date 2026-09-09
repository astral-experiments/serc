use crate::common;

#[test]
fn returns_copy_finally_bodies_before_leaving_the_enclosing_handler() {
    for scope in ["ordinary", "coroutine", "generator"] {
        let prefix = if scope == "coroutine" { "async " } else { "" };
        let await_prefix = if scope == "coroutine" { "await " } else { "" };
        let suspension = if scope == "generator" {
            "    yield None\n"
        } else {
            ""
        };
        for handler in ["except Error:", "except Error as error:", "except:"] {
            for value in ["None", "value", "run(value)"] {
                let value = if value == "run(value)" {
                    format!("{await_prefix}{value}")
                } else {
                    value.to_owned()
                };
                for finish in ["", "            finish(None)\n"] {
                    let source = format!(
                        "{prefix}def f(value, flag):\n    try:\n        {await_prefix}first(value)\n    {handler}\n        state = resource()\n        try:\n            prepare(state)\n            return {value}\n        finally:\n            {await_prefix}cleanup()\n{finish}{suspension}",
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_finally_return.py");
                }
            }
        }
    }

    let source = "def f(value):\n    try:\n        return work(value)\n    except Error:\n        state = resource()\n        try:\n            prepare(state)\n            return run(state, value)\n        finally:\n            state.close()\n            finish(None)\n";
    common::assert_matches_cpython_marshal(source, "handler_resource_finally_return.py");
}

#[test]
fn copied_finally_returns_discard_pending_values_before_outer_cleanup() {
    for binding in ["", " as error"] {
        for original in ["None", "value"] {
            for replacement in ["None", "value"] {
                let source = format!(
                    "def f(value, flag):\n    try:\n        work()\n    except Error{binding}:\n        try:\n            return {original}\n        finally:\n            if flag:\n                return {replacement}\n",
                );
                common::assert_matches_cpython_marshal(&source, "pending_handler_return.py");
            }
        }
    }

    for scope in ["ordinary", "coroutine", "async_with"] {
        let prefix = if scope == "ordinary" { "" } else { "async " };
        let initial = if scope == "ordinary" {
            ""
        } else {
            "    await initial()\n"
        };
        let manager_prefix = if scope == "async_with" { "async " } else { "" };
        for binding in ["", " as error"] {
            for value in ["None", "value", "run(value)"] {
                for conditional in [
                    "",
                    "                    if flag:\n                        return other\n",
                ] {
                    let source = format!(
                        "{prefix}def f(value, other, flag, values, manager, second_manager):\n{initial}    try:\n        first()\n    except Error{binding}:\n        for item in values:\n            {manager_prefix}with manager:\n                try:\n                    return {value}\n                finally:\n{conditional}                    cleanup()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "pending_manager_return.py");
                }
            }
        }
    }
}

#[test]
fn nested_retained_try_entries_use_fresh_truth_marker_slots() {
    let indent = |body: &str| format!("    {}", body.replace('\n', "\n    "));
    for captured in [false, true] {
        for condition in ["flag", "obj.ready"] {
            for prologue in ["", "obj.result = value\n", "obj.status = Status.DONE\n"] {
                for outer_finally in [false, true] {
                    let body = format!(
                        "{prologue}try:\n    if {condition}:\n        self.wait()\nexcept Error as exc:\n    raise\nreturn value\n",
                    );
                    let cleanup = if outer_finally {
                        "finally:\n    release()"
                    } else {
                        "except OuterError:\n    recover()"
                    };
                    let body = format!("try:\n{}\n{cleanup}\n", indent(&body));
                    let source = if captured {
                        let function = format!("def f(value):\n{}\nreturn f", indent(&body));
                        format!("def outer(obj, self, flag):\n{}\n", indent(&function))
                    } else {
                        format!("def f(value, obj, self, flag):\n{}\n", indent(&body))
                    };
                    common::assert_matches_cpython_marshal(&source, "retained_try_entry.py");
                }
            }
        }
    }
}

#[test]
fn handler_returns_keep_manager_and_finally_cleanup_owners() {
    for prefix in ["", "async "] {
        for value in ["None", "value", "call(value)"] {
            for binding in ["", " as error"] {
                for managers in ["first_manager", "first_manager, second_manager"] {
                    let source = format!(
                        "{prefix}def f(value, flag):\n    try:\n        with {managers}:\n            try:\n                work()\n            except Error{binding}:\n                prepare()\n                if flag:\n                    return {value}\n                use()\n        return {value}\n    finally:\n        cleanup()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_manager_return.py");
                }
            }
        }
    }
}

#[test]
fn normal_handler_cleanup_keeps_intervening_protected_regions() {
    for prefix in ["", "async "] {
        for body in [
            "try:\n    cancel()\nexcept Error:\n    pass\nfinally:\n    close()\nraise",
            "try:\n    raise outer_error\nfinally:\n    try:\n        refresh()\n    except Error as inner_error:\n        report(inner_error)",
        ] {
            let source = format!(
                "{prefix}def f():\n    try:\n        work()\n    except Error as outer_error:\n        {}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "intervening_handler_cleanup.py");
        }
    }
}

#[test]
fn nested_named_handler_returns_keep_the_outer_name_cleanup() {
    for prefix in ["", "async "] {
        for outer_binding in ["", " as outer_error"] {
            for value in ["None", "value", "call(value)"] {
                let source = format!(
                    "{prefix}def f(value):\n    try:\n        work()\n    except Error{outer_binding}:\n        try:\n            work()\n        except Error as inner_error:\n            return {value}\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_named_handler_return.py");
            }
        }
    }

    for manager in ["", "            with manager:\n"] {
        let body = if manager.is_empty() {
            "            try:\n                work()\n            except Error as inner_error:\n                return value\n            work()\n"
        } else {
            "                try:\n                    work()\n                except Error as inner_error:\n                    return value\n                work()\n"
        };
        let source = format!(
            "def f(value, manager):\n    try:\n        work()\n    except Error as outer_error:\n        try:\n            work()\n        except Error as middle_error:\n{manager}{body}        work()\n    work()\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_manager_return.py");
    }

    for prefix in ["", "async "] {
        let source = format!(
            "{prefix}def f(value):\n    try:\n        work()\n    except Error as outer_error:\n        try:\n            work()\n        except Error as middle_error:\n            try:\n                work()\n            except Error as inner_error:\n                return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_implicit_return.py");
        let source = format!("{source}    finish()\n");
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_normal_exit.py");
    }
}

#[test]
fn named_handler_return_clear_keeps_the_remaining_finally_owner() {
    for scope in ["ordinary", "coroutine", "generator", "async_generator"] {
        let prefix = if matches!(scope, "coroutine" | "async_generator") {
            "async "
        } else {
            ""
        };
        let suspension = if matches!(scope, "generator" | "async_generator") {
            "    yield None\n"
        } else {
            ""
        };
        for value in ["", " None", " value", " call(value)"] {
            if scope == "async_generator" && !value.is_empty() {
                continue;
            }
            let source = format!(
                "{prefix}def f(value, flag):\n{suspension}    try:\n        work()\n    except Error as error:\n        if flag:\n            return{value}\n        work()\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_return_clear.py");
            if scope == "ordinary" {
                let source =
                    source.replace("        if flag:\n            return", "        return");
                common::assert_matches_cpython_marshal(
                    &source,
                    "direct_named_handler_return_clear.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_named_handler_and_cleanup_locations() {
    let cases = [
        (
            "named_and_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_and_three_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            raise
    finish(value)
",
        ),
        (
            "named_and_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            return
    finish(value)
",
        ),
        (
            "named_and_three_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            return
    finish(value)
",
        ),
        (
            "unnamed_and_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error:
        if value.bad and value.ready:
            raise
    finish(value)
",
        ),
        (
            "named_or_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad or error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_single_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad:
            raise
    finish(value)
",
        ),
        (
            "named_and_nonterminal_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            consume(error)
    finish(value)
",
        ),
        (
            "named_and_followed_in_handler_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
        consume(error)
    finish(value)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
