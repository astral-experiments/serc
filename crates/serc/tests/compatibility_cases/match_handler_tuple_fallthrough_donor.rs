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

#[test]
fn matches_cpython_folded_match_tuple_return_cleanup_width() {
    for (filename, source) in [
        (
            "match_nested_join_tuple_with_assert_false_target.py",
            r#"def choose(mode, first, second, values):
    match mode:
        case None:
            if first:
                if second:
                    left = [VALUE]
                    right = "first"
                elif values:
                    left = [format_value(second)]
                    right = "second"
                else:
                    left = [VALUE]
                    right = "third"
            elif second:
                left = [VALUE]
                right = "fourth"
            else:
                report(values)
                left = [VALUE]
                right = "fifth"
        case "pair":
            if first:
                left = [VALUE]
                right = "pair-first"
            else:
                left = [VALUE]
                right = "pair-second"
        case "restore":
            left = [format_value(item) for item in values]
            right = "restored"
        case "plain":
            left = [VALUE]
            right = "plain"
        case _:
            assert False, mode
    return left, right
"#,
        ),
        (
            "match_nested_join_three_local_tuple_with_assert_false_control.py",
            r#"def choose(mode, first, second, values):
    match mode:
        case None:
            if first:
                if second:
                    left = [VALUE]
                    right = "first"
                elif values:
                    left = [format_value(second)]
                    right = "second"
                else:
                    left = [VALUE]
                    right = "third"
            elif second:
                left = [VALUE]
                right = "fourth"
            else:
                report(values)
                left = [VALUE]
                right = "fifth"
        case "pair":
            if first:
                left = [VALUE]
                right = "pair-first"
            else:
                left = [VALUE]
                right = "pair-second"
        case "restore":
            left = [format_value(item) for item in values]
            right = "restored"
        case "plain":
            left = [VALUE]
            right = "plain"
        case _:
            assert False, mode
    return left, right, values
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
