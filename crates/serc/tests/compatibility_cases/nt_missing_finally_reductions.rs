use crate::common;

#[test]
fn shared_normal_finally_with_nested_condition() {
    common::assert_matches_cpython_marshal(
        r"def process(value, path):
    try:
        store(value, path)
        actual = list(load(path))
        verify(actual, value)
    except Exception as error:
        report(error)
    finally:
        try:
            import storage as backend
            if backend.exists(path):
                backend.remove(path)
        except Exception:
            pass
",
        "imported_finally_condition.py",
    );
}

#[test]
fn terminal_handler_before_normal_finally() {
    common::assert_matches_cpython_marshal(
        r"def process(value, path):
    try:
        store(value, path)
    except Exception:
        raise
    finally:
        try:
            import storage as backend
            if backend.exists(path):
                backend.remove(path)
        except Exception:
            pass
",
        "terminal_handler_finally_condition.py",
    );
}

#[test]
fn shared_normal_finally_with_local_condition() {
    common::assert_matches_cpython_marshal(
        r"def process(value, ready):
    try:
        store(value)
    except Exception as error:
        report(error)
    finally:
        try:
            if ready:
                finish(value)
        except Exception:
            pass
",
        "local_finally_condition.py",
    );
}
