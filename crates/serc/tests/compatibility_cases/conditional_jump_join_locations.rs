use crate::common;

#[test]
fn matches_cpython_for_else_shared_branch_exit() {
    const SOURCE: &str = r"def f(flag, items, again, done):
    if flag:
        for item in items:
            if again:
                break
            work(item)
        else:
            if done:
                result = work()
    else:
        return fallback()
    after()
";
    for (filename, source) in [
        ("for_else_shared.py", SOURCE.to_owned()),
        (
            "for_else_and.py",
            SOURCE.replace("if done:", "if done and again:"),
        ),
        (
            "for_else_no_break.py",
            SOURCE.replace("            if again:\n                break\n", ""),
        ),
        (
            "for_else_return_control.py",
            SOURCE.replace("result = work()", "return work()"),
        ),
        (
            "for_else_raise_control.py",
            SOURCE.replace("result = work()", "raise Error()"),
        ),
        (
            "for_else_folded_control.py",
            SOURCE.replace("if done:", "if False:"),
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_with_shared_nested_branch_exit() {
    const SOURCE: &str = r"def f(items, flag, branch, value, seen):
    for item in items:
        with manager():
            if flag:
                if branch:
                    if seen:
                        ACTION
                else:
                    value = work()
                    if value is not None and seen:
                        ACTION
            else:
                after()
";
    for action in ["yield value", "work(value)", "result = value"] {
        for managers in ["manager()", "manager(), other_manager()"] {
            let source = SOURCE
                .replace("ACTION", action)
                .replace("manager()", managers);
            common::assert_matches_cpython_marshal(&source, "with_shared_nested_branch.py");
        }
    }
}
