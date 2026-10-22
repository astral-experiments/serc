def run(manager):
    with manager:
        try:
            raise ValueError
        except ValueError as exc:
            try:
                handle(exc)
            finally:
                value = None
    finish()
