def f(subject, manager, result, make):
    with manager:
        match subject:
            case [record]:
                result.value = make(record)
                result.cursor = record.cursor
            case _:
                raise ValueError(subject)
