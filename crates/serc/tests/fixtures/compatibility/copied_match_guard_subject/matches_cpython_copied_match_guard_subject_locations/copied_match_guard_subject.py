def f(value):
    match value:
        case number if 0 < number < 10:
            return Result.OK
        case number if number == 20:
            return Result.OTHER
        case _:
            return Result.FALLBACK
