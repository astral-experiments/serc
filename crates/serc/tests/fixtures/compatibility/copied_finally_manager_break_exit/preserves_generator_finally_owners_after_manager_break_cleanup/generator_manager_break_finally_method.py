def f(manager, resource, value):
    while True:
        try:
            with manager:
                yield value
                break
        except (ValueError, OSError):
            continue
        finally:
            if resource.exists():
                resource.unlink()
