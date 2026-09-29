def run(manager, value, release, finish):
    try:
        with manager:
            try:
                return None
            finally:
                release()
    finally:
        finish()
