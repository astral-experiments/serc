use crate::common;

#[test]
fn matches_cpython_assignment_cleanup_after_raising_alternative() {
    for handler in ["Error", "Error as error"] {
        for raising in ["raise", "raise Error(b)", "raise Error(b) from cause"] {
            let source = format!(
                "def f(a, b, items):\n    try:\n        value = work()\n    except {handler}:\n        if a:\n            report()\n            value = None\n        else:\n            report()\n            {raising}\n    return value\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "assignment_cleanup_after_raising_alternative.py",
            );
        }
    }
}

#[test]
fn matches_cpython_raising_alternative_in_enclosing_contexts() {
    for source in [
        r"def f(a, b, items):
    try:
        value = work()
    except Error:
        if a:
            report()
            value = None
        else:
            report()
            raise Error(b)
",
        r"async def f(a, b, items):
    try:
        value = work()
    except Error:
        if a:
            report()
            value = None
        else:
            report()
            raise Error(b)
    return value
",
        r"def f(a, b, items):
    with manager():
        try:
            value = work()
        except Error:
            if a:
                value = None
            else:
                report()
                raise Error(b) from cause
    return value
",
        r"def f(a, b, items):
    for item in items:
        try:
            value = work()
        except Error:
            if a:
                value = None
            else:
                report()
                raise Error(b) from cause
    return value
",
    ] {
        for handler in ["Error", "Error as error"] {
            let source = source.replace("except Error:", &format!("except {handler}:"));
            common::assert_matches_cpython_marshal(
                &source,
                "raising_alternative_in_enclosing_contexts.py",
            );
        }
    }
}
