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
