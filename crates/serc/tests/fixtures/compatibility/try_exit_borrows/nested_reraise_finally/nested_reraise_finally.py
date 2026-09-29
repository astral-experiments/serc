def f(value, emit):
    saved = value
    try:
        result = emit()
        try:
            result.wait()
            if result.failed:
                raise Failure()
        except Interrupt:
            result.stop()
            result.wait()
            raise
    finally:
        emit(saved)
