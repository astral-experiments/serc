use crate::common;

#[test]
fn matches_cpython_marshal_for_multi_edge_try_body_exits() {
    let source = r"def and_return(first, second):
    try:
        prepare()
        if first and second:
            return value()
    except Error:
        recover()
    after()

def and_raise(first, second):
    try:
        prepare()
        if first and second:
            raise Error
    except OtherError:
        recover()
    after()

def and_three_return(first, second, third):
    try:
        if first and second and third:
            return value()
    except Error:
        pass
    after()

def and_pass_handler(first, second):
    try:
        if first and second:
            return value()
    except Error:
        pass
    after()

def single_return(first):
    try:
        if first:
            return value()
    except Error:
        recover()
    after()

def or_return(first, second):
    try:
        if first or second:
            return value()
    except Error:
        recover()
    after()

def try_else(first, second):
    try:
        if first and second:
            return value()
    except Error:
        recover()
    else:
        success()
    after()
";
    common::assert_matches_cpython_marshal(source, "try_if_and_exit.py");
}
