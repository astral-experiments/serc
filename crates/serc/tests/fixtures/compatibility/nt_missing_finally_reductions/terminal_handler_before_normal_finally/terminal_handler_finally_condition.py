def process(value, path):
    try:
        store(value, path)
    except Exception:
        raise
    finally:
        try:
            import storage as backend
            if backend.exists(path):
                backend.remove(path)
        except Exception:
            pass
