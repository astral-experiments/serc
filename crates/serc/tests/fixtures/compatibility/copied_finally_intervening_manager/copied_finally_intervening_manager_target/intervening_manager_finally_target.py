def run(manager, value, release, finish):
    try:
        with manager:
            try:
                return value()
            finally:
                release()
    finally:
        finish()
