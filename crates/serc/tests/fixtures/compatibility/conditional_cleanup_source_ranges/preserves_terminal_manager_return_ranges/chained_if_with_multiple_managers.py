def inspect(low, value, high, first, second, emit):
    if low <= value < high:
        with first() as left, second() as right:
            emit(left, right, value)
