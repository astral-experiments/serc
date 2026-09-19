use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_finally_false_branch_owner_boundaries() {
    for tail in ["", "yield flag"] {
        for callee in ["obj", "obj.use"] {
            for branch in ["else:", "elif obj.flag:"] {
                let source = format!(
                    r"async def f(obj, flag):
    try:
        work()
    finally:
        if flag:
            pass
        {branch}
            await {callee}(1 if flag else 2)
    {tail}
"
                );
                assert_matches_cpython_marshal(&source, "false_branch_owner_boundary.py");
            }
        }
    }
}
