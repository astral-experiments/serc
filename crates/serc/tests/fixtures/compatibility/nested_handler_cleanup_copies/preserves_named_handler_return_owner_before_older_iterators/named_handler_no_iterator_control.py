def probe(value, run, recover, close):
    try:
        try:
            run(value)
        except KeyError as error:
            return recover(error)
    except RuntimeError:
        run(value)
    finally:
        close(value)
