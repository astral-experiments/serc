use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_nested_async_terminal_conditionals() {
    let source = r"async def conjunction(groups, left, right):
    async for group in groups:
        async for item in group:
            pass
    if left and right:
        raise ValueError
    return 1

async def disjunction(groups, left, right):
    async for group in groups:
        async for item in group:
            pass
    if left or right:
        raise ValueError
    return 1

async def adjacent_guards(self, groups, count, found):
    async for group in groups:
        async for item in group:
            pass
    if self.value is not None and count == 0:
        raise ValueError('missing')
    if self.value is not None and len(found) != 1:
        raise ValueError('multiple')
    return found
";
    assert_matches_cpython_marshal(source, "nested_async_terminal_conditionals.py");
}
