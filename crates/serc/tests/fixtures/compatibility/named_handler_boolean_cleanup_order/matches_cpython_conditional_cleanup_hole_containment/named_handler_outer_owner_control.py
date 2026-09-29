def inspect_value(read, fallback, finish):
    try:
        try:
            return read()
        except BaseException as error:
            if isinstance(error, LookupError):
                raise error
            elif isinstance(error, ValueError):
                if error.code == 7 and not fallback:
                    return None
            elif fallback:
                return read()
            else:
                raise error
    finally:
        finish()
