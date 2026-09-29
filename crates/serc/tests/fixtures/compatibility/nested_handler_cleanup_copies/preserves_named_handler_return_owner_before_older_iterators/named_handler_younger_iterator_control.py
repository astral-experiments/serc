def probe(values, run, recover, close):
    try:
        try:
            for value in values:
                try:
                    run(value)
                except KeyError as error:
                    return recover(error)
        except RuntimeError:
            run(None)
    finally:
        close(None)
