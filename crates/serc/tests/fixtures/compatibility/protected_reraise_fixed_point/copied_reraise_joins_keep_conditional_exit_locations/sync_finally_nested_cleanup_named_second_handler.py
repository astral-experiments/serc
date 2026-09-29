def run(fut):
    try:
        return work()
    finally:
        try:
            fut = start_cleanup()
            finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
        except Exception as error:
            warn(error)
