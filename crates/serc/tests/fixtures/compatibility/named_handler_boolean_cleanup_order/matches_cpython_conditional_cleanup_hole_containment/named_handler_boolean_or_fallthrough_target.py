def inspect_value(read, fallback):
    try:
        return read()
    except BaseException as error:
        if isinstance(error, LookupError):
            raise error
        elif isinstance(error, ValueError):
            if error.code == 7 or fallback:
                return None
        elif fallback:
            return read()
        else:
            raise error
