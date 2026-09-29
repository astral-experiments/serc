def probe(manager, pending, value):
    with manager():
        with manager():
            try:
                while pending:
                    work(pending)
            except BaseException:
                for future in pending:
                    future.cancel()
                raise
    consume(value)
    finish(value)
    return value
