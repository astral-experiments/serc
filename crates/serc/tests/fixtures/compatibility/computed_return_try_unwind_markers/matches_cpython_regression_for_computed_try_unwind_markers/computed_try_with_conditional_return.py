def read_last(manager, read, valid, ParseError, OuterError):
    try:
        with manager() as resource:
            try:
                value = read(resource)
                return value if valid(value) else None
            except ParseError:
                return None
    except OuterError:
        return None
