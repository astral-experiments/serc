use crate::common;

#[test]
fn matches_cpython_sourceless_and_located_try_else_exits() {
    for source in [
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            work()
        else:
            if b:
                other()
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            if b:
                work()
        else:
            if b:
                other()
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            work()
        elif b:
            other()
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a and b:
            return result
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            work()
        else:
            other()
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            if b:
                work()
        else:
            other()
    after()
",
        r"def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a or b:
            return result
    after()
",
        r"def f(a, b, c, result, items=()):
    try:
        work(result)
    except E:
        recover(result)
    else:
        if a and b:
            return result
    after(result)
",
    ] {
        common::assert_matches_cpython_marshal(source, "sourceless_try_else_exit.py");
    }
}

#[test]
fn shared_false_else_exit_in_nested_loop_try_stays_unlocated() {
    for condition in ["a and b", "a and b and c"] {
        let source = format!(
            "def f(items, a, b, c, value):\n    while a:\n        try:\n            prepare()\n        except OuterError:\n            pass\n        else:\n            try:\n                work()\n            except InnerError:\n                pass\n            else:\n                if {condition}:\n                    return value\n        advance()\n    return None\n"
        );
        common::assert_matches_cpython_marshal(&source, "shared_false_nested_try_else_exit.py");
    }
}
