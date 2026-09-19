use crate::common;

#[test]
fn preserves_manager_pop_owner_after_conditional_raise() {
    let source = r"def f(manager, value):
    with manager:
        try:
            consume(value)
            if value is not None:
                raise Error(value)
        except Error:
            recover()
            raise
    after()
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_raise.py");
}

#[test]
fn preserves_manager_pop_owner_after_conditional_return() {
    let source = r"async def f(manager, value):
    with manager:
        try:
            value = await read(value)
            if value is not None:
                report(value)
                return value
        except Error:
            recover()
            raise
    after()
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_return.py");
}

#[test]
fn preserves_manager_pop_owner_after_explicit_else() {
    let source = r"def f(manager, values):
    for value in values:
        with manager:
            try:
                if first(value) and second(value) and value:
                    report(value)
                    return value
                else:
                    report(value)
            except Error:
                recover()
                continue
";
    common::assert_matches_cpython_marshal(source, "et_normal_try_with_explicit_else.py");
}

#[test]
fn matches_cpython_manager_cleanup_pop_slot_owners() {
    for (filename, source) in [
        (
            "manager_generator_finally_augassign_pop_target.py",
            r"def f(manager, value, finish):
    with manager:
        try:
            yield value
        except ValueError:
            raise
        else:
            finish()
        finally:
            value -= 1
",
        ),
        (
            "manager_handler_retry_for_pop_target.py",
            r"def f(manager, inner, work, values, cleanup):
    with manager:
        try:
            with inner:
                work()
            return
        except ValueError:
            for value in values:
                try:
                    work(value)
                finally:
                    cleanup()
",
        ),
        (
            "manager_retry_break_finally_store_pop_target.py",
            r"def f(manager, values, work, update):
    with manager:
        for value in values:
            try:
                work(value)
                break
            except ValueError:
                work(None)
            finally:
                state = update(value)
        else:
            raise RuntimeError
    return state
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_unwind_pop_owners() {
    for (filename, source) in [
        (
            "conditional_value_return_protected_manager_pop_target.py",
            r"def run(manager, ready):
    with manager:
        try:
            while True:
                if step():
                    return 0 if ready else 1
        except StopIteration:
            return 2
",
        ),
        (
            "conditional_value_assignment_protected_manager_pop_control.py",
            r"def run(manager, ready):
    with manager:
        try:
            while True:
                if step():
                    result = 0 if ready else 1
                    return result
        except StopIteration:
            return 2
",
        ),
        (
            "copied_cleanup_terminal_raise_outer_finally_pop_target.py",
            r#"async def collect(items):
    try:
        try:
            await prepare()
            return build(), describe()
        finally:
            errors = []
            for item in items:
                try:
                    await item.close()
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup("cleanup failed", errors)
    finally:
        finish()
"#,
        ),
        (
            "copied_cleanup_following_call_outer_finally_pop_control.py",
            r#"async def collect(items):
    try:
        try:
            await prepare()
            return build(), describe()
        finally:
            errors = []
            for item in items:
                try:
                    await item.close()
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup("cleanup failed", errors)
            completed()
    finally:
        finish()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
