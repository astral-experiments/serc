use crate::common;

#[test]
fn matches_cpython_protected_terminating_branch_boolean_backedges() {
    for condition in ["flag and value is not None", "flag and ready(value)"] {
        for exit in ["return value", "continue", "break", "raise ValueError"] {
            for handler in ["pass", "continue", "observe(value)"] {
                let source = format!(
                    "def f(items, flag):\n    for value in items:\n        try:\n            observe(value)\n            if {condition}:\n                {exit}\n        except Exception:\n            {handler}\n    return None\n"
                );
                common::assert_matches_cpython_marshal(&source, "protected_tail_boolean.py");
            }
        }
    }
}

#[test]
fn matches_cpython_located_conditional_loop_backedges() {
    for (filename, source) in [
        (
            "conditional_false_retained_backedge_unprotected_control.py",
            r"def run(ready, poll, read, send, stop, report):
    while ready():
        if poll():
            data = read()
            if data:
                response = send(data)
                if response.status_code != 200:
                    report(response)
                    stop()
                    break
            else:
                break
",
        ),
        (
            "located_manager_break_without_outer_iterator_control.py",
            r"def run(item, manager, ready, choose, write, warn):
    while ready(item):
        if choose(item):
            with manager(item) as resource:
                write(resource)
            break
    else:
        warn(item)
",
        ),
        (
            "located_outer_iterator_backedge_after_manager_break_tail_control.py",
            r"def run(items, manager, ready, choose, write, warn, finish):
    for item in items:
        while ready(item):
            if choose(item):
                with manager(item) as resource:
                    write(resource)
                break
        else:
            warn(item)
        finish(item)
",
        ),
        (
            "located_outer_iterator_backedge_after_manager_break_target.py",
            r"def run(items, manager, ready, choose, write, warn):
    for item in items:
        while ready(item):
            if choose(item):
                with manager(item) as resource:
                    write(resource)
                break
        else:
            warn(item)
",
        ),
        (
            "normal_try_and_linear_break_independent_false_exits.py",
            r"def run(stop, check, prepare, work, notify, recover, finish):
    try:
        while True:
            try:
                try:
                    prepare()
                except ValueError:
                    recover()
                work()
                if stop and check():
                    notify()
                    break
            except Exception:
                recover()
    finally:
        finish()
",
        ),
        (
            "protected_conditional_false_populated_backedge_control.py",
            r"def run(ready, poll, read, send, stop, report, tick):
    try:
        while ready():
            if poll():
                data = read()
                if data:
                    response = send(data)
                    if response.status_code != 200:
                        report(response)
                        stop()
                        break
                else:
                    break
            tick()
    except Exception as error:
        report(error)
",
        ),
        (
            "protected_conditional_false_retained_backedge_target.py",
            r"def run(ready, poll, read, send, stop, report):
    try:
        while ready():
            if poll():
                data = read()
                if data:
                    response = send(data)
                    if response.status_code != 200:
                        report(response)
                        stop()
                        break
                else:
                    break
    except Exception as error:
        report(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
