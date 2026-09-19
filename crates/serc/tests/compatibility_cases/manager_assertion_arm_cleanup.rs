use crate::common;

#[test]
fn matches_cpython_manager_assertion_arm_cleanup_join() {
    let source = r#"def check(outer, inner, enabled, scores):
    with (
        outer,
        inner,
    ):
        if enabled:
            assert scores["value"] < 1.0, f"unexpected value: {scores['value']}"
        else:
            raise ValueError(f"disabled: {enabled}")
"#;
    common::assert_matches_cpython_marshal(source, "manager_assertion_arm_cleanup_join.py");
}

#[test]
fn matches_cpython_manager_match_terminal_try_exit() {
    for (filename, source) in [
        (
            "manager_match_terminal_try_target.py",
            r"async def f(value, manager, work, report, reject):
    with manager:
        match value:
            case First():
                try:
                    return await work(value)
                except:
                    report(value)
                    raise
            case Last():
                return await work(value)
            case _:
                reject(value)
",
        ),
        (
            "manager_match_direct_return_control.py",
            r"async def f(value, manager, work, report, reject):
    with manager:
        match value:
            case First():
                return await work(value)
            case Last():
                return await work(value)
            case _:
                reject(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_manager_assertion_operands() {
    for (filename, source) in [
        (
            "manager_assert_constant_ifexp_condition_control.py",
            r"def verify(manager, value, expected):
    with manager():
        assert (
            value == expected
            if True
            else False
        )
",
        ),
        (
            "manager_assert_folded_ifexp_false_arm_target.py",
            r#"def verify(manager, value, expected, choose):
    with manager():
        assert (
            value == expected
            if choose
            else ""
        )
"#,
        ),
        (
            "manager_assert_folded_or_false_arm_target.py",
            r"def verify(manager, value, expected):
    with manager():
        assert (
            value == expected
            or False
        )
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
