def inspect(low, value, high, manager, emit):
    if low <= value < high:
        with manager(
            "selected",
            expanded=True,
        ) as selected:
            emit(selected, value)
