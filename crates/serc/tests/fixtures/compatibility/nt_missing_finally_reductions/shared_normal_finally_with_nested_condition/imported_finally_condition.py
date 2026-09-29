def process(value, path):
    try:
        store(value, path)
        actual = list(load(path))
        verify(actual, value)
    except Exception as error:
        report(error)
    finally:
        try:
            import storage as backend
            if backend.exists(path):
                backend.remove(path)
        except Exception:
            pass
