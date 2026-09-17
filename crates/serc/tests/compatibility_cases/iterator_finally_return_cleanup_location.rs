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
