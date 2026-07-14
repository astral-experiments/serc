use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_multi_with_raise_exits() {
    let source = r"def nested_raise(outer, first, second, error):
    with outer:
        with first, second:
            raise error
    verify(error)

def nested_call_raise(outer, first, second, error):
    with outer():
        with first(), second():
            raise error
    verify(error)

def nested_asserted_raise(outer, first, second, error):
    with outer as value:
        with first, second:
            raise error
    assert value is error
";
    common::assert_matches_cpython_marshal(source, "nested_multi_with_raise_exits.py");
}
