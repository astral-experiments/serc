use crate::common;

#[test]
fn tuple_genexpr_keyword_return_async_target() {
    let source = r"
async def final_keyword(values, enabled):
    return Result(
        items=(
            tuple(value for value in values)
            if enabled
            else ()
        ),
    )


async def penultimate_keyword(values, enabled, count):
    return Result(
        items=(
            tuple(value for value in values if value is not None)
            if enabled
            else ()
        ),
        count=count,
    )
";
    common::assert_matches_cpython_marshal(source, "tuple_genexpr_keyword_return_async.py");
}

#[test]
fn tuple_genexpr_keyword_return_sync_control() {
    let source = r"
def final_keyword(values, enabled):
    return Result(
        items=(
            tuple(value for value in values)
            if enabled
            else ()
        ),
    )


def penultimate_keyword(values, enabled, count):
    return Result(
        items=(
            tuple(value for value in values if value is not None)
            if enabled
            else ()
        ),
        count=count,
    )
";
    common::assert_matches_cpython_marshal(source, "tuple_genexpr_keyword_return_sync.py");
}
