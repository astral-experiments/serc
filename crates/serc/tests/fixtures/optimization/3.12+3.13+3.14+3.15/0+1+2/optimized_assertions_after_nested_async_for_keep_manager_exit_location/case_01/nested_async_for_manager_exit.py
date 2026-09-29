async def f(outer, inner, items):
    with outer:
        with inner:
            results: list[Item] = []
            async for item in items():
                results.append(item)
            assert len(results) == 1
            assert results[0].value
