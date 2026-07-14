use crate::common;

#[test]
fn matches_cpython_marshal_for_try_else_call_exit_locations() {
    let source = r"def direct(flag):
    try:
        action()
    except Error:
        pass
    else:
        if check(flag):
            return result
    consume()


def raised(flag):
    try:
        action()
    except Error:
        pass
    else:
        if check(flag):
            raise Problem
    consume()


def final_elif(flag):
    try:
        action()
    except Error:
        pass
    else:
        if first_check(flag):
            return first_result
        elif second_check(flag):
            return second_result
    consume()


def multiline_method(state, flag):
    try:
        action()
    except FirstError:
        pass
    except SecondError:
        pass
    else:
        if state.ready(
            flag,
        ):
            return result
    consume()
";
    common::assert_matches_cpython_marshal(source, "try_else_call_exit_locations.py");
}
