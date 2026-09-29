def run(first, read, reraise):
    try:
        yield read()
    except ValueError as error:
        if first:
            try:
                yield read()
            except TypeError as nested:
                if reraise:
                    raise nested
                yield read()
        else:
            yield read()
