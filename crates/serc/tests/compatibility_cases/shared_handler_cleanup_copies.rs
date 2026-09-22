use crate::common::assert_matches_cpython_marshal;

#[test]
fn finalizer_false_edges_keep_their_own_cleanup_locations() {
    let source = include_str!("../fixtures/finalizer_false_handler_cleanup.py");
    for condition in [
        "current() and ready()",
        "(task := current()) and task.ready()",
        "current is not None and ready()",
    ] {
        let source = source.replace("current() and ready()", condition);
        assert_matches_cpython_marshal(&source, "finalizer_false_handler_cleanup.py");
    }
}

#[test]
fn nested_named_handler_join_gets_a_private_return_copy() {
    let source = include_str!("../fixtures/nested_named_handler_cleanup.py");
    for value in ["[item.name for item in gather()]", "gather()"] {
        let source = source.replace("[item.name for item in gather()]", value);
        assert_matches_cpython_marshal(&source, "nested_named_handler_cleanup.py");
    }
}

#[test]
fn finalizer_cleanup_keeps_single_edge_and_named_handler_boundaries() {
    let source = include_str!("../fixtures/finalizer_false_handler_cleanup.py");
    for source in [
        source.replace("current() and ready()", "ready()"),
        source.replace("except RuntimeError:", "except RuntimeError as error:"),
    ] {
        assert_matches_cpython_marshal(&source, "finalizer_handler_cleanup_boundaries.py");
    }
}
