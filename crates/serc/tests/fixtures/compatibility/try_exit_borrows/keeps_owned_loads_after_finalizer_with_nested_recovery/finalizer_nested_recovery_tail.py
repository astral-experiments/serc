def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        if selected is not None and proc.poll() is None:
            proc.terminate()
        if proc.stdout:
            proc.stdout.close()
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
