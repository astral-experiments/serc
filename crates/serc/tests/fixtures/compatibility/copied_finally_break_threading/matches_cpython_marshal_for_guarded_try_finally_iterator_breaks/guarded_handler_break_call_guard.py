def transfer(resource, attempts, limit, perform, already_done):
    last_error = None
    for attempt in attempts:
        try:
            perform()
            return
        except Exception as error:
            if already_done(error):
                return
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource.exists():
                try:
                    resource.close()
                except OSError:
                    pass
    assert last_error is not None
    raise last_error
