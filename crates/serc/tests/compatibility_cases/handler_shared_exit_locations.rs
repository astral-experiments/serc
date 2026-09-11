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
