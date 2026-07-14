use crate::common;

#[test]
fn matches_cpython_marshal_for_direct_return_debug() {
    let source = r"def direct():
    return __debug__


def after_branch(value):
    if value is not None:
        return value
    return __debug__


def same_line_multiline():
    return (__debug__
    )


async def coroutine():
    return __debug__


def generator():
    yield None
    return __debug__


def loop(flag):
    while flag:
        return __debug__
    return False
";

    common::assert_matches_cpython_marshal(source, "direct_return_debug.py");
}
