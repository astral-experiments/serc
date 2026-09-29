def assertions(first, second):
    assert first.value < second.value
    assert (
        first.value
        <= second.value
    )
    assert first.value != second.value


def branches(first, second, predicate):
    if first.value == second.value:
        consume(first)
    if (
        first.value
        >= second.value
    ):
        consume(second)
    if predicate(first, second):
        consume(first, second)
