def with_suffix(items, left, right, consume):
    for value in items:
        if consume(value):
            assert value
        else:
            if left(value):
                assert value
            elif right(value):
                assert value
            else:
                assert value
            assert value


def without_suffix(items, left, right, consume):
    for value in items:
        if consume(value):
            assert value
        else:
            if left(value):
                assert value
            elif right(value):
                assert value
            else:
                assert value
