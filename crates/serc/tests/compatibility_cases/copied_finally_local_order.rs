use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_local_order() {
    let source = r"def conditional_return(flag):
    try:
        if flag:
            return flag
        later = 1
    finally:
        cleanup = 2

async def return_before_handler(work):
    try:
        result = await work()
        return result
    except Exception as error:
        report(error)
        raise
    finally:
        elapsed = timer()
        report(elapsed)
";
    common::assert_matches_cpython_marshal(source, "copied_finally_local_order.py");
}
