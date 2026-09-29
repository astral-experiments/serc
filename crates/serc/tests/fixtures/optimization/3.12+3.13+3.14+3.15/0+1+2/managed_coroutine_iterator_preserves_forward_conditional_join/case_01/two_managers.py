async def f(items, boundary, first, second):
    with first, second:
        for item in items:
            values = await receive(item)
            value = values[0]
            if item >= boundary:
                assert value.valid
            else:
                assert not value.valid
            if item <= boundary:
                assert value.ready
            else:
                assert value.other
