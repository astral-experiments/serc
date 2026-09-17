use crate::common;

#[test]
fn matches_cpython_marshal_for_match_handler_tuple_fallthrough_donor() {
    let source = r"def select_sizes(divisor, pages, limit):
    try:
        left, right = lookup(divisor)
    except LookupError:
        kind = get_kind()
        if kind < 4:
            raise ValueError('unsupported')
        match kind:
            case 4:
                left, right = (512 // divisor, 32)
            case 7:
                left, right = (4096 // divisor, 32)
            case _:
                left, right = (2048 // divisor, 32)
    left, right = (min(pages, left), min(limit, right))
    return left, right
";
    common::assert_matches_cpython_marshal(source, "match_handler_tuple_fallthrough_donor.py");
}
