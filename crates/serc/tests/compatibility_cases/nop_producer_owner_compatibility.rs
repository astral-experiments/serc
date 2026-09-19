use crate::common;

#[test]
fn matches_cpython_sync_with_trailing_constant_owner() {
    for (filename, source) in [
        (
            "sync_with_trailing_constant.py",
            "with manager:\n    work()\n    '''comment\n    spanning lines\n    '''\n",
        ),
        (
            "nested_sync_with_trailing_constant.py",
            "def f(manager):\n    try:\n        with manager:\n            work()\n            'comment'\n    except Exception:\n        recover()\n",
        ),
        (
            "nested_if_constant_keeps_owner.py",
            "with manager:\n    if condition:\n        work()\n        'comment'\n",
        ),
        (
            "async_with_constant_keeps_owner.py",
            "async def f(manager):\n    async with manager:\n        work()\n        'comment'\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nop39_assert_false_handler() {
    let source = r#"def f(items):
    for item in items:
        try:
            item()
        except Exception:
            assert (
                False
            ), "failed"
"#;
    common::assert_matches_cpython_marshal(source, "nop39_assert_false_handler.py");
}

#[test]
fn matches_cpython_nop39_final_pass_under_with() {
    let source = r"for group in groups:
    with manager:
        for item in group:
            if item:
                work()
                pass
            followup()
";
    common::assert_matches_cpython_marshal(source, "nop39_final_pass_under_with.py");
}

#[test]
fn matches_cpython_nop39_async_constant_condition() {
    let source = r"async def f(check, pause, work):
    while True:
        await pause()
        if (
            True
            and check()
        ):
            work()
";
    common::assert_matches_cpython_marshal(source, "nop39_async_constant_condition.py");
}

#[test]
fn matches_cpython_nop39_direct_sync_with_tail_pass() {
    for (source, filename) in [
        (
            "def f(manager, result):\n    with manager:\n        first()\n        second()\n        third()\n        pass\n    return result\n",
            "direct_sync_with_tail.py",
        ),
        (
            "with first_manager, second_manager:\n    work()\n    pass\n",
            "nested_sync_with_tail.py",
        ),
        (
            "def f(manager):\n    yield 0\n    with manager:\n        work()\n        pass\n    yield 1\n",
            "generator_sync_with_tail.py",
        ),
        (
            "try:\n    prepare()\nfinally:\n    with manager:\n        work()\n        pass\n",
            "finally_sync_with_tail.py",
        ),
        (
            "async def f(manager):\n    async with manager:\n        work()\n        pass\n",
            "async_with_tail_negative.py",
        ),
        (
            "with manager:\n    work()\n    pass\nfollowup()\n",
            "nonterminal_sync_with_tail.py",
        ),
        (
            "with manager:\n    work()\n    pass\n    pass\n",
            "two_pass_sync_with_tail.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
