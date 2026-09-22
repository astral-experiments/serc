use crate::common::assert_matches_cpython_marshal;

#[test]
fn copies_expression_discards_before_original_try_backedges() {
    for (source, filename) in [
        (
            include_str!("../fixtures/normal_try_ifexp_discard_tail.py"),
            "normal_try_ifexp_discard_tail.py",
        ),
        (
            include_str!("../fixtures/normal_try_tuple_discard_tail.py"),
            "normal_try_tuple_discard_tail.py",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copies_expression_discard_before_located_try_continuation() {
    for (source, filename) in [
        (
            include_str!("../fixtures/normal_try_ifexp_discard_continuation.py"),
            "normal_try_ifexp_discard_continuation.py",
        ),
        (
            include_str!("../fixtures/normal_try_tuple_discard_continuation.py"),
            "normal_try_tuple_discard_continuation.py",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
