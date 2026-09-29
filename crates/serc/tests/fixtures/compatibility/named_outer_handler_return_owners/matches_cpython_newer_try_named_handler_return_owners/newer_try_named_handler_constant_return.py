def run(first, second, flag):
    try:
        try:
            first()
        except LookupError as error:
            try:
                second()
                if flag:
                    return None
                return False
            except ValueError:
                raise
    except RuntimeError:
        return 1
