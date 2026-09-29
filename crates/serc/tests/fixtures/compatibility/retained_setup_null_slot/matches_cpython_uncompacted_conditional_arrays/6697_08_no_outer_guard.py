try:
    with manager():
        data = read(path)
    if not data:
        warn()
    else:
        size = None
        try:
            if path.startswith("prefix:") and helper is not None:
                size = first()
            else:
                size = second()
        except Exception:
            pass
        report(size)
except Exception:
    report(None)
