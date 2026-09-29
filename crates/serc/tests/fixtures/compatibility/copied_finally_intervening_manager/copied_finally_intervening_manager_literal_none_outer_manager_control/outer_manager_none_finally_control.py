def run(manager, value, release, finish):
    with manager:
        try:
            try:
                return None
            finally:
                release()
        finally:
            finish()
