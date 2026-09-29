def run(manager, value, release, finish):
    with manager:
        try:
            try:
                return value()
            finally:
                release()
        finally:
            finish()
