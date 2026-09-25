use crate::common;

#[test]
fn matches_cpython_all_generator_multiline() {
    let source = r"def f(flag, values):
    return (
        all(x for x in values)
        if flag
        else False
    )
";
    common::assert_matches_cpython_marshal(source, "all_generator_multiline.py");
}

#[test]
fn matches_cpython_any_generator_multiline() {
    let source = r"def f(flag, values):
    return (
        any(x for x in values)
        if flag
        else False
    )
";
    common::assert_matches_cpython_marshal(source, "any_generator_multiline.py");
}

#[test]
fn matches_cpython_ternary_generator_multiline() {
    let source = r"def f(flag, values):
    return (
        tuple(x for x in values)
        if flag
        else ()
    )
";
    common::assert_matches_cpython_marshal(source, "ternary_generator_multiline.py");
}

#[test]
fn matches_cpython_generic_generator_multiline() {
    let source = r"def f(flag, fn, values):
    return (
        fn(x for x in values)
        if flag
        else ()
    )
";
    common::assert_matches_cpython_marshal(source, "generic_generator_multiline.py");
}

#[test]
fn matches_cpython_list_generator_multiline() {
    let source = r"def f(flag, values):
    return (
        list(x for x in values)
        if flag
        else []
    )
";
    common::assert_matches_cpython_marshal(source, "list_generator_multiline.py");
}

#[test]
fn matches_cpython_shadowed_tuple_generator_multiline() {
    let source = r"def f(flag, values, tuple):
    return (
        tuple(x for x in values)
        if flag
        else ()
    )
";
    common::assert_matches_cpython_marshal(source, "shadowed_tuple_generator_multiline.py");
}

#[test]
fn matches_cpython_tuple_generator_same_line() {
    let source = r"def f(flag, values):
    return tuple(x for x in values) if flag else ()
";
    common::assert_matches_cpython_marshal(source, "tuple_generator_same_line.py");
}

#[test]
fn matches_cpython_tuple_generator_false_arm() {
    let source = r"def f(flag, values):
    return (
        ()
        if flag
        else tuple(x for x in values)
    )
";
    common::assert_matches_cpython_marshal(source, "tuple_generator_false_arm.py");
}

#[test]
fn matches_cpython_bookkeeping_boundary_for_try_star_pass() {
    let source = r"def f(result):
    try:
        observe()
    except* Exception:
        pass
    return result
";
    common::assert_matches_cpython_marshal(source, "try_star_pass.py");
}

#[test]
fn matches_cpython_bookkeeping_boundary_for_try_star_named_pass() {
    let source = r"def f(result):
    try:
        observe()
    except* Exception as error:
        pass
    return result
";
    common::assert_matches_cpython_marshal(source, "try_star_named_pass.py");
}

#[test]
fn matches_cpython_terminal_assignment_with_tuple_generator_join() {
    let cases = [
        r"result = (
    tuple(f(x) for x in g())
    if cond == value
    else ()
)
",
        "result = tuple(x for x in values) if flag else ()\n",
        "result = all(x for x in values) if flag else ()\n",
        "result = any(x for x in values) if flag else ()\n",
        "def f(flag, values):\n    result = tuple(x for x in values) if flag else ()\n",
        "result = () if flag else tuple(x for x in values)\n",
        "result = tuple(x for x in values) if flag else ()\nconsume(result)\n",
        "target.result = tuple(x for x in values) if flag else ()\n",
    ];
    for source in cases {
        common::assert_matches_cpython_marshal(source, "terminal_tuple_generator_assignment.py");
    }
}
