use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_primitive_tuple_finally_owners() {
    for value in [
        "()",
        "('token_0',)",
        "('token_0', 'token_1')",
        "('token_0', 'token_1', 'token_2')",
    ] {
        let source = format!(
            "def f(flag, other_flag, manager):
    try:
        x = work({value})
    finally:
        if flag is None:
            cleanup()
        else:
            other()
"
        );
        assert_matches_cpython_marshal(&source, "primitive_tuple_finally_owners.py");
    }
}

#[test]
fn matches_cpython_marshal_for_tuple_folds_before_finally_scopes() {
    let source = "def f(flag, other_flag, manager):
    x = work(('token_0',))
    try:
        work(flag)
    finally:
        if flag is None:
            cleanup()
        else:
            other()
";
    assert_matches_cpython_marshal(source, "tuple_fold_before_finally.py");
}

#[test]
fn matches_cpython_marshal_for_tuple_folds_with_nested_finally_owners() {
    let source = "def f(flag, other_flag, manager):
    try:
        try:
            x = work(('token_0', 'token_1'))
        finally:
            if flag is None:
                cleanup()
            else:
                other()
    finally:
        finish()
";
    assert_matches_cpython_marshal(source, "tuple_fold_nested_finally.py");
}

#[test]
fn preserves_cpython_marshal_for_other_finally_conditions() {
    let source = "def f(flag, other_flag, manager):
    try:
        with manager:
            x = work(('token_0', 'token_1', 'token_2', 'token_3'))
    finally:
        if flag is None:
            cleanup()
        else:
            other()
";
    assert_matches_cpython_marshal(source, "larger_tuple_finally.py");

    for condition in ["flag", "flag is None"] {
        let source = format!(
            "def f(flag):
    try:
        work()
    finally:
        if {condition}:
            cleanup()
"
        );
        assert_matches_cpython_marshal(&source, "finally_without_tuple_fold.py");
    }
}
