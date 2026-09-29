def run(lock, again):
    try:
        while again():
            first, second, third, fourth = values()
            try:
                work()
            finally:
                with lock:
                    touch()
                del first
                del second
    finally:
        finish()
