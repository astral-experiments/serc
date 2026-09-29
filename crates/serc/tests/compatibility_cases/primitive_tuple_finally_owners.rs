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

#[test]
fn preserves_primitive_binary_fold_slots_before_try_finally() {
    for literal in ["'0' * 64", "b'x' * 8", "17 + 25", "3 << 2"] {
        let source = format!(
            "def f(codec, flag):
    header = pack({literal})
    first = prepare()
    second = other()
    third = lookup()
    try:
        value = codec.dumps(first)
        output = codec.dumps(second)
    finally:
        if not flag:
            codec.restore(third)
    return value, output, header
"
        );
        assert_matches_cpython_marshal(&source, "primitive_binary_before_finally.py");
    }
}

#[test]
fn preserves_primitive_binary_fold_slots_inside_try_finally() {
    for literal in ["'left' + 'right'", "b'x' * 8", "17 + 25", "3 << 2"] {
        for condition in ["flag", "flag is None", "flag is not None"] {
            let source = format!(
                "def f(flag):
    try:
        value = work({literal})
        work()
    finally:
        if {condition}:
            cleanup()
    return value
"
            );
            assert_matches_cpython_marshal(&source, "primitive_binary_inside_finally.py");
        }
    }
}

#[test]
fn keeps_binary_fold_owner_proofs_separate_at_source_joins() {
    for expression in [
        "'x' * 8",
        "('x' * 4) + 'tail'",
        "('x'\n                * 8)",
    ] {
        let source = format!(
            "def f(flag, other):
    try:
        if flag:
            value = work({expression})
        else:
            value = other()
    finally:
        if other is not None:
            cleanup()
    return value
"
        );
        assert_matches_cpython_marshal(&source, "binary_fold_source_join_controls.py");
    }
}
