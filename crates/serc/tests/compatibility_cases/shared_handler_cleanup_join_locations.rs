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
