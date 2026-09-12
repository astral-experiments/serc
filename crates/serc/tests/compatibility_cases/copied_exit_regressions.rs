use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_call_original_boundary() {
    let source = r#"def final_else_target(branch, condition, left, right, sink):
    if branch:
        sink(left)
    else:
        sink(left if condition else right)


def tested_branch_after_region_target(work, branch, condition, left, right, sink):
    try:
        work()
    except Exception:
        pass
    if branch:
        sink(left if condition else right)


def generator_argument_target(instance, items):
    if items is not None:
        object.__setattr__(instance, "items", tuple(item for item in items))


def direct_call_control(condition, left, right, sink):
    sink(left if condition else right)
"#;
    common::assert_matches_cpython_marshal(source, "terminal_call_original_boundary.py");
}

#[test]
fn matches_cpython_marshal_for_normal_finally_call_original_jump() {
    let source = r"def normal_finally_target(work, condition, left, right, sink):
    try:
        work()
    finally:
        sink(left if condition else right)


def normal_finally_following_control(work, condition, left, right, sink, after):
    try:
        work()
    finally:
        sink(left if condition else right)
    after()
";
    common::assert_matches_cpython_marshal(source, "normal_finally_call_original_jump.py");
}

#[test]
fn matches_cpython_marshal_for_exceptional_finally_inherited_restriction() {
    let source = r"def target(work, cleanup, after, finish):
    try:
        try:
            work()
        finally:
            try:
                cleanup()
            except Exception:
                pass
        after()
    except Exception:
        pass
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "exceptional_finally_inherited_restriction.py");
}

#[test]
fn matches_cpython_marshal_for_async_exceptional_finally_inherited_restriction() {
    let source = r"async def target(work, cleanup, after, finish):
    try:
        try:
            await work()
        finally:
            try:
                await cleanup()
            except Exception:
                after()
        after()
    except Exception:
        pass
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(
        source,
        "async_exceptional_finally_inherited_restriction.py",
    );
}

#[test]
fn matches_cpython_marshal_for_exceptional_finally_without_inherited_restriction() {
    let source = r"def control(work, cleanup, after):
    try:
        try:
            work()
        finally:
            try:
                cleanup()
            except Exception:
                pass
        after()
    except Exception:
        pass
";
    common::assert_matches_cpython_marshal(
        source,
        "exceptional_finally_without_inherited_restriction.py",
    );
}
