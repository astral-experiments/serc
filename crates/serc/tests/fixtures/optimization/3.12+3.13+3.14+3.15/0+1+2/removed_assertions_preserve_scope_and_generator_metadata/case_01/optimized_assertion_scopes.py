
def assigned():
    assert (local := value)
    return local
def outer(captured):
    def inner():
        assert captured
    return inner
def captured_by_assert(local):
    assert lambda: local
def generator():
    assert (yield value)
def delegated():
    assert (yield from values)
async def async_generator():
    assert (yield value)
async def coroutine():
    assert await value
