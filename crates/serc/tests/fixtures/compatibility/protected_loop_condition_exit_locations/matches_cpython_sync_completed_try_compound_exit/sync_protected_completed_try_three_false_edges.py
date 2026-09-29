def inspect_values(items, depth, emit):
    try:
        for value in items:
            try:
                try:
                    text = str(value)
                except Exception as error:
                    emit(error)
                if depth < 4 and hasattr(value, "name") and any(part in value.name for part in ("red", "blue")):
                    emit(value)
            except Exception as error:
                emit(error)
    except Exception as error:
        emit(error)
