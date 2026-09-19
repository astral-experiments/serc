use crate::common;

#[test]
fn matches_cpython_shared_handler_exit_locations() {
    for (filename, source) in [
        (
            "two_false_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and value:
                report(error)
                continue
        finish(value)
",
        ),
        (
            "two_false_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and value:
                report(error)
                break
        finish(value)
",
        ),
        (
            "one_false_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag:
                report(error)
                continue
        finish(value)
",
        ),
        (
            "one_false_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag:
                report(error)
                break
        finish(value)
",
        ),
        (
            "or_one_false_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag or value:
                report(error)
                continue
        finish(value)
",
        ),
        (
            "or_one_false_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag or value:
                report(error)
                break
        finish(value)
",
        ),
        (
            "fold_false_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and False:
                report(error)
                continue
        finish(value)
",
        ),
        (
            "fold_false_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and False:
                report(error)
                break
        finish(value)
",
        ),
        (
            "fold_true_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if True and flag:
                report(error)
                continue
        finish(value)
",
        ),
        (
            "fold_true_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if True and flag:
                report(error)
                break
        finish(value)
",
        ),
        (
            "nested_bare_two_assign.py",
            r"def f(values, flag):
    for value in values:
        try:
            try:
                work()
            except Exception:
                raw = read()
                if raw:
                    result = transform(raw)
                    value = raw
            if flag:
                finish(value)
        except Exception:
            recover()
",
        ),
        (
            "split_loop_continue.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and value:
                report(error)
                continue
        if flag:
            finish(value)
",
        ),
        (
            "split_loop_break.py",
            r"def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and value:
                report(error)
                break
        if flag:
            finish(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_unnamed_handler_nested_shared_returns() {
    for (filename, source) in [
        (
            "unnamed_handler_nested_assignment_shared_return_target.py",
            r"def select_value(value, first, second):
    result = value
    try:
        check(value)
    except ValueError:
        if first:
            if second:
                result = value.primary
            else:
                result = value.secondary
        else:
            result = value.fallback
    return result
",
        ),
        (
            "unnamed_handler_nested_try_shared_return_target.py",
            r#"def select_value(value, first):
    result = {}
    try:
        check(value)
    except ValueError:
        if first:
            try:
                result["value"] = convert(value)
            except TypeError:
                result["value"] = None
        else:
            result["value"] = value
    return result
"#,
        ),
        (
            "handler_manager_shared_local_return_target.py",
            r"def read_cached(kind, open_resource, load, build, save):
    try:
        if kind == 'text':
            with open_resource('r') as handle:
                result = load(handle)
        elif kind == 'bytes':
            with open_resource('rb') as handle:
                result = load(handle)
        else:
            raise ValueError('unknown format')
    except FileNotFoundError:
        result = build()
        if kind == 'text':
            with open_resource('w') as handle:
                save(result, handle)
        elif kind == 'bytes':
            with open_resource('wb') as handle:
                save(result, handle)
        else:
            raise ValueError('unknown format')
    return result
",
        ),
        (
            "nested_fallback_manager_shared_checked_return_target.py",
            r"def read_fallback(open_resource, read, report):
    try:
        with open_resource('primary') as handle:
            result = read(handle)
    except ValueError:
        try:
            with open_resource('fallback') as handle:
                result = read(handle)
        except ValueError:
            report()
            result = []
    return result
",
        ),
        (
            "unnamed_handler_nested_assignment_none_return_control.py",
            r"def update_value(value, first, second):
    try:
        check(value)
    except ValueError:
        if first:
            if second:
                value.primary = 1
            else:
                value.secondary = 2
        else:
            value.fallback = 3
    return None
",
        ),
        (
            "unnamed_handler_nested_cleanup_small_return_control.py",
            r"def recover(fetch, report, first, second):
    result = None
    try:
        result = fetch()
    except Exception:
        if first:
            if second:
                report()
            else:
                report()
        else:
            report()
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_unnamed_handler_original_cleanup_width() {
    for (filename, source) in [
        (
            "unnamed_terminal_pass_cleanup_width_target.py",
            r"async def recover(flag):
    try:
        await work()
    except LookupError:
        if flag:
            first()
        else:
            second()
        pass
",
        ),
        (
            "unnamed_terminal_conditional_cleanup_width_control.py",
            r"async def recover(flag):
    try:
        await work()
    except LookupError:
        if flag:
            first()
        else:
            second()
",
        ),
        (
            "unnamed_suppression_checked_return_width_target.py",
            r"def recover(flag, manager):
    try:
        result = work()
    except LookupError:
        if flag:
            with manager:
                raise ValueError()
        else:
            raise TypeError()
    return result
",
        ),
        (
            "unnamed_fallthrough_manager_return_width_control.py",
            r"def recover(flag, manager):
    try:
        result = work()
    except LookupError:
        if flag:
            with manager:
                result = repair()
        else:
            raise TypeError()
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
