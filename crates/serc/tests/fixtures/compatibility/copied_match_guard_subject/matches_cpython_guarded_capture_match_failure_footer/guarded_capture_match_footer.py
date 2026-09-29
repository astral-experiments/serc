def f(flag, other, value):
    if flag:
        match value:
            case number if number > 1:
                return Result.OK
            case number if number <= 1:
                return Result.OTHER
    elif other:
        return Result.ELSE
    return Result.FALLBACK
