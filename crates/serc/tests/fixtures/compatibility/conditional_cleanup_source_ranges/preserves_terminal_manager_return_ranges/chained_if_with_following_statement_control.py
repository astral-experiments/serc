def inspect(low, value, high, manager, emit):
    if low <= value < high:
        with manager():
            emit(value)
    emit(high)
