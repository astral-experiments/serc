use crate::common;

#[test]
fn matches_cpython_marshal_for_cell_prologue_owner_vacancies() {
    // Prologue insertions consume vacancies before the finally-condition marker.
    for captured in ["None", "(flag,)", "(flag, a,)", "(flag, a, b, c,)"] {
        for calls in 0..8 {
            let body = if calls == 0 {
                "        pass\n".to_owned()
            } else {
                "        work()\n".repeat(calls)
            };
            let source = format!(
                "def f(flag, a, b, c):\n    def capture():\n        return {captured}\n    try:\n{body}    finally:\n        if flag is None:\n            cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "cell_prologue_owner_vacancies.py");
        }
    }
}

#[test]
fn matches_cpython_marshal_for_free_prologue_and_cell_owner_boundaries() {
    for source in [
        r"def f(flag, a, b, c, d):
    def capture():
        return flag
    try:
        result = work((0,1,2))
    finally:
        if flag is None:
            cleanup()
",
        r"def f(flag):
    def capture():
        return flag
    if SENTINEL:
        prepare()
    try:
        work()
        work()
        work()
        work()
    finally:
        if flag is None:
            cleanup()
",
        r"def outer(flag):
    def f():
        try:
            work()
            work()
            work()
            work()
        finally:
            if flag is None:
                cleanup()
    return f
",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "free_prologue_and_cell_owner_boundaries.py",
        );
    }
}
