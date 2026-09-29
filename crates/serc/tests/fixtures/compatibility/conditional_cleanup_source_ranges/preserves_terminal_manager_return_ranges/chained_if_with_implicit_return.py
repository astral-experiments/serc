def inspect(index, values, manager, emit):
    if 0 <= int(index) < len(values):
        with manager("selected", expanded=True):
            emit(values[int(index)])
