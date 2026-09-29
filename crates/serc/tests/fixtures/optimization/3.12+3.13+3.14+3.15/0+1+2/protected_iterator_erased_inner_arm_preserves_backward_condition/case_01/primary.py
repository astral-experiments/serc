def f(manager, items, kind, cached, transform, dispatch):
    with manager:
        for item in items:
            if kind == 0:
                dispatch(item)
            elif kind == 1 or kind == 2:
                item['tag'] = 'value'
                if cached(item):
                    assert item
                else:
                    if transform(item):
                        item['value'] = transform(item)
                    item['value'] = dispatch[0](item)
            else:
                assert item
        dispatch(items)
