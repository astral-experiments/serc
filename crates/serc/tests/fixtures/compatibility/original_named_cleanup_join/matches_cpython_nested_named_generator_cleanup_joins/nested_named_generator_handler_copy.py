def run(first, read, reraise):
    try:
        yield read()
    except ValueError as error:
        if first:
            try:
                yield read()
            except TypeError as error:
                if reraise:
                    raise error
                yield read()
        else:
            yield read()
