def f(values, make):
    for value in values:
        resource = None
        try:
            resource = make(value)
            resource.prepare()
            resource.bind(value)
            return value
        except OSError:
            continue
        finally:
            try:
                if resource is not None:
                    resource.close()
            except Exception:
                pass
    return None
