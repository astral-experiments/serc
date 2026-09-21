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

#[test]
fn matches_cpython_conditional_jump_after_unconditional_exits() {
    common::assert_matches_cpython_marshal(
        r"def finalizer(outer, inner, call):
    if outer:
        if inner:
            before()
            try:
                call()
            finally:
                after()
    else:
        alternate()
    finish()

def exhaustive_match(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
    else:
        value = outer
    finish(value)

def following_work(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
            consume(value)
    else:
        value = outer
    finish(value)
",
        "conditional_jump_after_unconditional_exits.py",
    );
}
