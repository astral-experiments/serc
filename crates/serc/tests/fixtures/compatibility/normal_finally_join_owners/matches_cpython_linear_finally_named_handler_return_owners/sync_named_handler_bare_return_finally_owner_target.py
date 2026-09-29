def run(worker, token, recover, fatal):
    try:
        try:
            worker.start(token)
        except LookupError as error:
            recover(error)
            if fatal:
                raise ValueError("failed") from error
            return
        except RuntimeError:
            recover(None)
            raise
        finally:
            worker.finish(token)
    except OSError:
        recover(None)
    return None
