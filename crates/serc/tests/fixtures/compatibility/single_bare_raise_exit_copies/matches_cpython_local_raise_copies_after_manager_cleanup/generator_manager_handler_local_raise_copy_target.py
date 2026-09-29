def f(manager):
    while True:
        try:
            yield work()
        except Exception as error:
            with manager:
                record(error)
            raise error
        finally:
            with manager:
                finish()
        break
