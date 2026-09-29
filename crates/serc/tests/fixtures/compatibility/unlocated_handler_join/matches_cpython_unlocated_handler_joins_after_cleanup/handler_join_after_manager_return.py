def read(load, manager, cached):
    try:
        value = load()
    except ValueError:
        with manager:
            return cached
    except Exception:
        with manager:
            return cached
    return value
