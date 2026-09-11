use crate::common;

#[test]
fn matches_cpython_final_pattern_loop_failure_locations() {
    let cases = [
        (
            "match_failure_direct_none_continue.py",
            r"def f(items):
    for item in items:
        match item:
            case Entry(value=value):
                consume(value)
            case None:
                continue
    return None
",
        ),
        (
            "match_failure_nested_if_continue.py",
            r"def f(xs, flag, other):
    for x in xs:
        if flag:
            if other:
                match x:
                    case 0:
                        return x
                    case None:
                        continue
            else:
                continue
    return None
",
        ),
        (
            "match_failure_try_for.py",
            r"def f(xs):
    for x in xs:
        try:
            match x:
                case 0:
                    observe(x)
                case Kind.VALUE:
                    observe(x)
        except Error:
            recover()
",
        ),
        (
            "match_failure_async_for_body.py",
            r"async def f(xs):
    for x in xs:
        try:
            match x:
                case 0:
                    await observe(x)
                case Kind.VALUE:
                    await observe(x)
        except Error:
            recover()
",
        ),
        (
            "match_failure_while_tail.py",
            r"def f(xs):
    while xs:
        x = take(xs)
        try:
            match x:
                case 0:
                    observe(x)
                case Kind.VALUE:
                    observe(x)
        except Error:
            recover()
",
        ),
        (
            "match_failure_wildcard_negative.py",
            r"def f(xs, flag):
    for x in xs:
        if flag:
            match x:
                case 0:
                    observe(x)
                case _:
                    observe(x)
",
        ),
        (
            "match_failure_following_statement_negative.py",
            r"def f(xs, flag):
    for x in xs:
        if flag:
            match x:
                case 0:
                    observe(x)
                case None:
                    observe(x)
            finish()
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
