
assert [item for item in values]
class C:
    assert [item for item in values]
def check(values):
    assert [item for item in values]
    after = 1
    return after
async def async_check(values):
    assert [item async for item in values]
def lexical_before(values):
    item = 1
    assert [item for item in values]
    return item
def lexical_after(values):
    assert [item for item in values]
    after = 1
    item = 2
    return (after, item)
def captured_comprehension(values):
    assert [lambda: item for item in values]
def asserted_local_reference():
    assert later
    first = 1
    later = 2
    return (first, later)
def unused_assertion_binding():
    assert (unused := 1)
    used = 2
    return used
