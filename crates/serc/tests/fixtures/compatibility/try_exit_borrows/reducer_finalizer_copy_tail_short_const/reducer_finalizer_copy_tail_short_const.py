def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = 0
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
