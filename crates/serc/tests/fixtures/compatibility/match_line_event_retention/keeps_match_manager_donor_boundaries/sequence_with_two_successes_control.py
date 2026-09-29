def f(subject, manager, result, make):
    with manager:
        match subject:
            case [record]:
                result.value = make(record)
                result.cursor = record.cursor
            case [left, right]:
                result.value = left
                result.cursor = right
            case _:
                raise ValueError(subject)
