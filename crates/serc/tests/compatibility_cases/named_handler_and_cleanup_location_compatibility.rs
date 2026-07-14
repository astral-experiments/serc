use crate::common;

#[test]
fn matches_cpython_marshal_for_named_handler_and_cleanup_locations() {
    let cases = [
        (
            "named_and_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_and_three_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            raise
    finish(value)
",
        ),
        (
            "named_and_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            return
    finish(value)
",
        ),
        (
            "named_and_three_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            return
    finish(value)
",
        ),
        (
            "unnamed_and_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error:
        if value.bad and value.ready:
            raise
    finish(value)
",
        ),
        (
            "named_or_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad or error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_single_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad:
            raise
    finish(value)
",
        ),
        (
            "named_and_nonterminal_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            consume(error)
    finish(value)
",
        ),
        (
            "named_and_followed_in_handler_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
        consume(error)
    finish(value)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
