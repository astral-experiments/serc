use crate::common;

#[test]
fn matches_cpython_conditional_jump_shared_disjunction_join_location() {
    let source = r"def probe(selector, outer, left, right):
    if outer and (left or right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
";
    common::assert_matches_cpython_marshal(source, "conditional_jump_shared_join.py");
}

#[test]
fn matches_cpython_conditional_jump_single_predecessor_join_location() {
    let source = r"def probe(selector, outer, left, right):
    if outer and (right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
";
    common::assert_matches_cpython_marshal(source, "conditional_jump_single_join.py");
}
