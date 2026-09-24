use crate::common;

#[test]
fn matches_cpython_authored_pass_join_locations() {
    let source = include_str!("../fixtures/authored_pass_join_locations.py");
    common::assert_matches_cpython_marshal(source, "authored_pass_join_locations.py");
}

#[test]
fn matches_cpython_omitted_pass_at_normal_try_exit() {
    for (source, filename) in [
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "named_handler_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error:
        recover(item)
    observe(item)
    consume(item)
    finish(item)
",
            "unnamed_handler_footer.py",
        ),
        (
            r"def f(item):
    try:
        item = work(item)
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "assigned_call_footer.py",
        ),
        (
            r"def f(item):
    try:
        try:
            work(item)
            pass
        except Error as exc:
            recover(exc)
        observe(item)
    finally:
        finish(item)
    consume(item)
",
            "nested_finally_footer.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_omitted_pass_with_small_exit_and_existing_markers() {
    for (source, filename) in [
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error as exc:
        recover(exc)
    return None
",
            "short_return_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item); pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "same_line_pass_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item)
        42
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "constant_then_pass.py",
        ),
        (
            r"def f(item):
    try:
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "pass_only_footer.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn chained_comparison_pass_arms_keep_original_jump_boundaries() {
    for (filename, source) in [
        (
            "async_nested_chained_pass_assignment.py",
            r"async def run(fetch, low, high, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                previous = value
        output.append(previous)
    return output
",
        ),
        (
            "async_nested_chained_pass_call_assignment.py",
            r"async def run(fetch, low, high, reject, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                reject(value)
                previous = value
        output.append(previous)
    return output
",
        ),
        (
            "sync_boolean_chained_pass_elif_pass.py",
            r"def run(skip_empty, skip_busy, completed, failed, busy, report):
    if skip_empty and completed == failed == 0:
        pass
    elif skip_busy and not busy:
        pass
    else:
        report()
    return completed, failed
",
        ),
        (
            "chained_pass_small_return_control.py",
            r"def run(a, b, c, fallback):
    if a < b < c:
        pass
    else:
        fallback()
    return None
",
        ),
        (
            "negated_chained_pass_control.py",
            r"def run(a, b, c, fallback, output):
    if not a < b < c:
        pass
    else:
        fallback()
    output.append(a)
    return output
",
        ),
        (
            "same_line_chained_pass_control.py",
            r"def run(a, b, c, fallback, output):
    if a < b < c: pass
    else:
        fallback()
    output.append(a)
    return output
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_assertion_success_authored_pass_join() {
    let source = r"async def validate(mode, selected, left, right):
    if mode == 0:
        result = left
        assert left
        pass
    elif mode == 1:
        result = right
        if selected:
            assert len(left) == len(right)
        else:
            assert len(left) == 0
        pass
    else:
        raise ValueError(mode)
    assert result
";
    common::assert_matches_cpython_marshal(
        source,
        "assertion_success_authored_pass_join_target.py",
    );
}

#[test]
fn matches_cpython_declaration_only_conditional_suites() {
    for (filename, source) in [
        (
            "declaration_only_global_if_target.py",
            r"def inspect_data(enabled, debug, values):
    if enabled:
        if debug:
            global PROCESSED_VALUES
        if set(values) == {1}:
            return values
    return None
",
        ),
        (
            "declaration_only_nonlocal_if_target.py",
            r"def outer():
    value = 0
    def inspect_data(enabled):
        if enabled:
            nonlocal value
        return value
    return inspect_data
",
        ),
        (
            "declaration_only_protected_global_target.py",
            r"def inspect_data(enabled, touch, recover):
    try:
        if enabled:
            global SHARED_VALUE
        touch()
    except ValueError:
        recover()
",
        ),
        (
            "declaration_only_global_elif_else_target.py",
            r"def inspect_data(first, second):
    if first:
        global FIRST_VALUE
    elif second:
        global SECOND_VALUE
    else:
        global THIRD_VALUE
    return 7
",
        ),
        (
            "declaration_only_nonlocal_else_target.py",
            r"def outer():
    value = 0
    def inspect_data(enabled, touch):
        if enabled:
            touch()
        else:
            nonlocal value
        return value
    return inspect_data
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_final_conditional_arm_pass_before_cleanup() {
    for (filename, source) in [
        (
            "both_pass.py",
            r"def f(flag):
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            last()
            pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "else_both.py",
            r"def f(flag):
    try:
        if flag == 1:
            first()
            pass
        else:
            last()
            pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "finally_pass.py",
            r"def f(flag):
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            last()
            pass
    finally:
        cleanup()
    return flag
",
        ),
        (
            "same_line.py",
            r"def f(flag):
    try:
        if flag == 1:
            first(); pass
        elif flag == 2:
            last(); pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "await_arm.py",
            r"async def f(flag):
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            await last()
            pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "generator_both.py",
            r"def f(flag):
    yield flag
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            last()
            pass
    except Error as error:
        recover(error)
    finally:
        cleanup()
    return flag
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_omitted_pass_before_nested_branch_jump() {
    for (filename, source) in [
        (
            "nested_join_before_pass.py",
            r"def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
            pass
        elif flag == 2:
            last()
            pass
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "head_async.py",
            r"async def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
            pass
        elif flag == 2:
            last()
            pass
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "head_no_pass.py",
            r"def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
        elif flag == 2:
            last()
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "head_following_call.py",
            r"def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
            pass
            observe()
        elif flag == 2:
            last()
            pass
            observe()
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return flag
",
        ),
        (
            "head_single_return.py",
            r"def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
            pass
        elif flag == 2:
            last()
            pass
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_noop_finally_on_loop_backedges() {
    for (filename, source) in [
        (
            "pass_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "pass_handler_literal_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            42\n",
        ),
        (
            "call_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            x()\n        finally:\n            pass\n",
        ),
        (
            "bare_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "named_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception as error:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "else_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        else:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "while_pass_handler_pass_finally.py",
            "def f(x, y):\n    while y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "pass_handler_two_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n            pass\n",
        ),
        (
            "generator_pass_handler_pass_finally.py",
            "def f(x, y):\n    yield\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "coroutine_pass_handler_pass_finally.py",
            "async def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "async_for_pass_handler_pass_finally.py",
            "async def f(x, y):\n    async for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n",
        ),
        (
            "no_handler_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        finally:\n            pass\n",
        ),
        (
            "pass_handler_following_pass_finally.py",
            "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except Exception:\n            pass\n        finally:\n            pass\n        x()\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_noop_loop_finally_after_raising_handlers() {
    for handler in ["raise", "raise TypeError()"] {
        for exception in ["Exception", "Exception as error"] {
            let source = format!(
                "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except {exception}:\n            {handler}\n        finally:\n            pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "raising_handler_pass_finally.py");
        }
    }
}
