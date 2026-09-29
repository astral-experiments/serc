def f(items, emit):
    try:
        while True:
            result = emit(items)
            if result.pending:
                continue
            if result.failed:
                return 2
            rows = result.rows or []
            break
    except Exception as exc:
        emit(exc)
        return 2
    emit("done")
    emit(len(rows))
    return 0
