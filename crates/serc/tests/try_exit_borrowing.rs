#[allow(dead_code)]
mod common;

#[test]
fn nested_recovery_blocks_later_conditional_borrow_entry() {
    for inner in ["if other:\n                    use(value)", "use(value)"] {
        let source = format!(
            "def f(flag, other, value):\n    try:\n        if flag:\n            try:\n                {inner}\n            except Exception:\n                handle()\n            if other:\n                value = 1\n            else:\n                value = 2\n    except Exception:\n        handle()\n    use(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_recovery_borrow.py");
    }
}

#[test]
fn returning_conditional_keeps_branching_assignment_exit_empty() {
    for value in ["value", "value or default", "value if third else default"] {
        let source = format!(
            "def f(first, second, third, value):\n    try:\n        if first and second:\n            return value\n        elif first and third:\n            value = {value}\n    except Exception as error:\n        handle(error)\n        value = fallback\n    if value:\n        use(value)\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "returning_conditional_borrow.py");
    }
}

#[test]
fn nested_conditional_alternative_reaches_normal_finally() {
    let source = r"def f(items, first, second, third, value):
    for item in items:
        try:
            if first:
                if second:
                    value += 1
                    if third:
                        use(value)
                else:
                    value = 0
                    use(value)
            else:
                value = 0
        except Exception as error:
            handle(error)
        finally:
            if value > limit:
                use(value)
";
    common::assert_matches_cpython_marshal(source, "nested_conditional_finally.py");
}

#[test]
fn manager_iterator_continue_blocks_cleanup_loop_borrowing() {
    for suffix in ["use(item)", "await use(item)"] {
        let source = format!(
            "async def f(items, flag, value):\n    try:\n        with manager():\n            for item in items:\n                try:\n                    use(item)\n                except Exception:\n                    handle()\n                    continue\n                {suffix}\n    finally:\n        while flag:\n            await use(value)\n        use(value)\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "manager_continue_finally.py");
    }
}

#[test]
fn borrowed_loads_follow_original_try_exits() {
    let source = r"def after_else(value, first, second):
    try:
        read()
    except ValueError:
        pass
    else:
        if first:
            if second:
                return 0
            else:
                return 1
    report(value)

def after_finally(items, value):
    try:
        read()
    finally:
        for item in items:
            try:
                process(item)
            except ValueError:
                continue
            record(item)
    report(value)

def after_recovery(value, retry, stop):
    try:
        value = read()
    except ValueError:
        while retry:
            if stop:
                break
            else:
                record()
        value = fallback()
    report(value)

def in_manager(manager, value, stop):
    with manager:
        try:
            value = read()
        except ValueError as error:
            if stop:
                raise
            record(error)
        finally:
            report(value)
";
    common::assert_matches_cpython_marshal(source, "borrowed_try_exits.py");
}

#[test]
fn guarded_handler_exits_reach_normal_cleanup() {
    let source = "\
def run(value, retry, failed):
    try:
        work(value)
    except Error as error:
        if retry:
            log(error)
        if failed:
            raise
        log(value)
    finally:
        notify(value)
        value.close()
";
    common::assert_matches_cpython_marshal(source, "guarded_handler_exits.py");
}
