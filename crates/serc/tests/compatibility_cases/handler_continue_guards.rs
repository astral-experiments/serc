use crate::common::assert_matches_cpython_marshal;

#[test]
fn handler_continue_none_guard() {
    assert_matches_cpython_marshal(
        r"def cleanup(items, manager, acquire, finish):
    for item in items:
        with manager:
            resource = None
            try:
                resource = acquire(item)
                resource.start()
            except Exception:
                continue
            finally:
                if resource is not None:
                    resource.close(wait=True)
            finish(item)
",
        "handler_continue_none_guard.py",
    );
}

#[test]
fn handler_continue_named_none_guard_control() {
    assert_matches_cpython_marshal(
        r"def cleanup(items, manager, acquire, finish):
    for item in items:
        with manager:
            resource = None
            try:
                resource = acquire(item)
                resource.start()
            except Exception as error:
                continue
            finally:
                if resource is not None:
                    resource.close(wait=True)
            finish(item)
",
        "handler_continue_named_none_guard_control.py",
    );
}

#[test]
fn handler_continue_truth_guard_control() {
    assert_matches_cpython_marshal(
        r"def cleanup(items, manager, acquire, finish):
    for item in items:
        with manager:
            resource = None
            try:
                resource = acquire(item)
                resource.start()
            except Exception:
                continue
            finally:
                if resource:
                    resource.close(wait=True)
            finish(item)
",
        "handler_continue_truth_guard_control.py",
    );
}
