use crate::common;

#[test]
fn matches_cpython_marshal_for_conditional_handler_cleanup_order() {
    let source = r"def terminal_named_handler(value):
    try:
        work()
    except RuntimeError as error:
        if value:
            pass
        else:
            raise

def named_handler_with_following_code(value):
    try:
        work()
        update()
    except RuntimeError as error:
        report(error)
        if value:
            recover()
            result = value
        else:
            raise
    finish()

def bare_handler_with_following_code(value):
    try:
        work()
    except RuntimeError:
        if value:
            recover()
        else:
            raise
    finish()

def generator_handler(value):
    yield value
    try:
        work()
    except RuntimeError as error:
        if value:
            recover()
        else:
            raise
";
    common::assert_matches_cpython_marshal(source, "conditional_handler_cleanup_order.py");
}

#[test]
fn matches_cpython_nested_handler_fallthrough_cleanup() {
    for capture in ["", " as error"] {
        let source = format!(
            "def f(value):\n    try:\n        work(value)\n    except Error{capture}:\n        try:\n            return value\n        except Error:\n            pass\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_handler_fallthrough_cleanup.py");
    }
}
