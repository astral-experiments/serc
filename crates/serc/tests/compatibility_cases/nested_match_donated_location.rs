use crate::common::assert_matches_cpython_marshal;

#[test]
fn nested_match_branch_join_inherits_copied_exit_location() {
    assert_matches_cpython_marshal(
        r"def process(kind, outer, middle, inner, value):
    match kind:
        case 'first':
            if outer:
                if middle:
                    if inner:
                        consume(value)
                else:
                    alternate(value)
        case _:
            fallback(value)
    finish(value)
",
        "nested_match_donated_location.py",
    );
}
