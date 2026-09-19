use crate::common;

#[test]
fn original_finally_source_alias_recovery_control() {
    let source = r"def f(flag, primary, secondary, observe, cleanup):
    try:
        if flag:
            raise RuntimeError
        primary()
    except ValueError:
        try:
            secondary()
            observe()
        except TypeError:
            pass
    finally:
        cleanup()
";
    common::assert_matches_cpython_marshal(source, "original_finally_source_alias_recovery.py");
}

#[test]
fn original_finally_source_alias_recovery_live_tail_control() {
    let source = r"def f(flag, primary, secondary, observe, cleanup):
    try:
        if flag:
            raise RuntimeError
        primary()
    except ValueError:
        try:
            secondary()
            observe()
        except TypeError:
            pass
        observe()
    finally:
        cleanup()
";
    common::assert_matches_cpython_marshal(
        source,
        "original_finally_source_alias_recovery_live_tail.py",
    );
}
