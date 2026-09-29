def inspect_values(items, depth, emit):
    try:
        for value in items:
            try:
                try:
                    text = str(value)
                except Exception as error:
                    emit(error)
                if depth < 4:
                    emit(value)
            except Exception as error:
                emit(error)
    except Exception as error:
        emit(error)
