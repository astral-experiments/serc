def run(resource, mode, emit, value):
    def capture():
        return value
    try:
        if mode:
            emit(mode)
        elif mode is None:
            emit(None)
        else:
            emit(resource)
            if resource is not None:
                try:
                    resource.unlink()
                except FileNotFoundError:
                    pass
    except Exception:
        emit("error")
        raise
    return value
