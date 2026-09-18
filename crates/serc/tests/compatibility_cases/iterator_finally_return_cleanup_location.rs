use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_iterator_finally_return_cleanup_location() {
    let source = r"def select(groups, cleanup, state):
    try:
        for group in groups:
            try:
                validate(group)
                if group is None:
                    return state.missing
                for item in group:
                    if item:
                        return state.found
            finally:
                cleanup(group)
    except ValueError:
        return state.failed
    return state.empty
";
    assert_matches_cpython_marshal(source, "iterator_finally_return_cleanup_location.py");
}

#[test]
fn matches_cpython_iterator_finally_return_without_outer_handler_control() {
    let source = r"def select(groups, cleanup, state):
    for group in groups:
        try:
            validate(group)
            if group is None:
                return state.missing
            for item in group:
                if item:
                    return state.found
        finally:
            cleanup(group)
    return state.empty
";
    assert_matches_cpython_marshal(source, "iterator_finally_return_without_outer_handler.py");
}

#[test]
fn shared_guarded_cleanup_keeps_iterator_discard_unlocated() {
    let source = r"def run(values, accepted, exists, remove, client):
    try:
        for value in values:
            try:
                if accepted(value):
                    return
            finally:
                if value and exists(value):
                    try:
                        remove(value)
                    except OSError:
                        pass
    finally:
        try:
            client.disconnect()
        except OSError:
            pass
";
    assert_matches_cpython_marshal(
        source,
        "guarded_pass_cleanup_iterator_bare_return_location_target.py",
    );
}
