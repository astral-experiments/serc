use crate::common;

#[test]
fn matches_cpython_shared_handler_cleanup_join_locations() {
    for (filename, source) in [
        (
            "unnamed_elif.py",
            r"def f(work, a, b, after):
    try:
        work()
    except ValueError:
        if a:
            raise
        elif b:
            value = work()
    after()
",
        ),
        (
            "guarded_assert_finally.py",
            r#"def f(work, flag, after):
    try:
        work()
    except Exception as error:
        if flag:
            assert work(error), "failed"
    finally:
        after()
"#,
        ),
        (
            "nested_partial_if_else.py",
            r"def f(work, flag, cached, after):
    try:
        return work()
    except NameError:
        if flag:
            value = work()
            if cached:
                return value
            else:
                value = work()
                work(value)
    after()
",
        ),
        (
            "nested_if_assignment_finally.py",
            r"async def f(work, outer, middle, inner, after):
    try:
        await work()
    except Exception as error:
        value = error
        if outer:
            if middle:
                if inner:
                    work()
                else:
                    work(error)
                work()
            value = False
    finally:
        after()
",
        ),
        (
            "guarded_assert_without_message_control.py",
            r"def f(work, flag, after):
    try:
        work()
    except Exception as error:
        if flag:
            assert work(error)
    finally:
        after()
",
        ),
        (
            "unnamed_elif_loop_tail_control.py",
            r"def f(items, strict, silent, call):
    for item in items:
        try:
            call(item)
        except Exception:
            if strict:
                raise
            elif not silent:
                call(item)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_handler_cleanup_donor_locations() {
    for (filename, source) in [
        (
            "annotation_handler.py",
            r#"def f(work):
    try:
        return work()
    except AttributeError:
        error: str
    except ZeroDivisionError:
        error = "failed"
    work(error)
"#,
        ),
        (
            "guarded_assert_false.py",
            r#"def f(work, fail, after):
    try:
        work()
    except Exception as error:
        if not fail:
            assert False, f"unexpected: {error}"
    else:
        work()
    after()
"#,
        ),
        (
            "valued_annotation_handler_control.py",
            r#"def f(work):
    try:
        return work()
    except AttributeError:
        error: str = "attribute"
    except ZeroDivisionError:
        error = "zero"
    work(error)
"#,
        ),
        (
            "guarded_false_assert_with_control.py",
            r"def f(work, manager, flag):
    with manager:
        try:
            work()
        except Exception as error:
            if not flag:
                assert False, error
        work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_shared_false_cleanup_contexts() {
    for asynchronous in [false, true] {
        for context in ["plain", "with", "finally"] {
            for raises in [false, true] {
                for following in [false, true] {
                    let function = if asynchronous { "async def" } else { "def" };
                    let prefix = if context == "with" {
                        "        "
                    } else {
                        "    "
                    };
                    let exit = if raises {
                        "raise"
                    } else if asynchronous {
                        "return await work()"
                    } else {
                        "return work()"
                    };
                    let manager = if context == "with" {
                        "    with manager:\n"
                    } else {
                        ""
                    };
                    let cleanup = if context == "finally" {
                        "    finally:\n        finish()\n"
                    } else {
                        ""
                    };
                    let after = if following { "    after()\n" } else { "" };
                    let source = format!(
                        "{function} f(a, b, manager):\n{manager}{prefix}try:\n{prefix}    work()\n{prefix}except Exception as error:\n{prefix}    prepare()\n{prefix}    if a and b:\n{prefix}        {exit}\n{cleanup}{after}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "named_false_cleanup_context.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_final_elif_for_exhaustion_cleanup() {
    const SOURCE: &str = r#"def f(first, second, items):
    try:
        work()
    except Error as error:
        if first:
            return first
        elif second:
            for item in items:
                value = work(item)
                if value is not None:
                    return value
    raise Error()
"#;

    for (filename, source) in [
        ("named_for_return.py", SOURCE.to_owned()),
        (
            "unnamed_for_return.py",
            SOURCE.replace("except Error as error:", "except Error:"),
        ),
        ("async_function_for_return.py", format!("async {SOURCE}")),
        (
            "for_continue.py",
            SOURCE.replace("return value", "continue"),
        ),
        (
            "prefix_call.py",
            SOURCE.replace(
                "            for item",
                "            before()\n            for item",
            ),
        ),
        (
            "for_break_control.py",
            SOURCE.replace("return value", "break"),
        ),
        (
            "loop_else_return_control.py",
            SOURCE.replace(
                "    raise Error()",
                "            else:\n                return fallback()\n    raise Error()",
            ),
        ),
        (
            "folded_elif_control.py",
            SOURCE.replace("elif second:", "elif True:"),
        ),
        (
            "nonterminating_first_control.py",
            SOURCE.replace("return first", "work(first)"),
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_nested_handler_cleanup_prefix_locations() {
    let indent = |source: &str| {
        source
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect::<String>()
    };
    for depth in 1..=4 {
        for named in [false, true] {
            for guarded in [false, true] {
                let mut body = "pass\n".to_owned();
                for level in 0..depth {
                    let binding = if named {
                        format!(" as error_{level}")
                    } else {
                        String::new()
                    };
                    body = format!(
                        "try:\n    work()\nexcept Error{binding}:\n{}else:\n    return work()\n",
                        indent(&body)
                    );
                }
                if guarded {
                    body = format!("if flag:\n{}", indent(&body));
                }
                let source = format!("def f(flag):\n{}    return fallback()\n", indent(&body));
                common::assert_matches_cpython_marshal(&source, "nested_handler_cleanup_prefix.py");
                if depth == 3 {
                    common::assert_matches_cpython_marshal(
                        &source.replace("return work()", "work()"),
                        "shared_handler_cleanup_prefix_control.py",
                    );
                }
            }
        }
    }
}
