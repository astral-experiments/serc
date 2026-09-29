def probe(values, run, retry, recover, close):
    for value in values:
        try:
            try:
                run(value)
            except KeyError as error:
                if retry(error):
                    continue
                return recover(error)
        except RuntimeError:
            run(value)
        finally:
            close(value)
