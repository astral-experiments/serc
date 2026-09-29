async def f(value, emit, done):
    task = None
    result = None
    error = None
    try:
        try:
            task = await start(value)
        except BaseError as exc:
            error = exc
            task = recover(value)
    finally:
        if task is not None:
            while True:
                if done:
                    break
                try:
                    result = await shield(task)
                    break
                except Cancel as exc:
                    if task.cancelled():
                        raise
                    if error is None:
                        error = exc
                except Error as exc:
                    if error is not None:
                        raise error from exc
                    raise
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
