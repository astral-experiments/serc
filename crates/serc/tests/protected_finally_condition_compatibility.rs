#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_protected_finally_condition_owners() {
    let source = r"def assert_compare_cleanup(manager, proc):
    with manager:
        try:
            proc.wait()
            assert proc.returncode == 1
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait()
    return proc

def attribute_cleanup(manager, value):
    with manager:
        prepare(value)
        try:
            result = load(value)
        finally:
            if value.locked:
                unlock(value)
    return result

def name_cleanup_control(manager, value, locked):
    with manager:
        try:
            result = load(value)
        finally:
            if locked:
                unlock(value)
    return result
";
    common::assert_matches_cpython_marshal(source, "protected_finally_condition_owners.py");
}
