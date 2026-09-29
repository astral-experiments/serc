def bare_cleanup():
    try:
        yield source
    except:
        raise
    finally:
        cleanup()


def named_cleanup(trace):
    try:
        yield trace
    except Exception as error:
        trace.set_error(error)
        raise
    finally:
        trace.reset()


def assigned_cleanup(limit):
    try:
        executor = make_executor(limit)
        yield executor
    except KeyboardInterrupt:
        clear_workers()
        raise
    finally:
        executor.shutdown(wait=False)


def attributed_cleanup(client):
    try:
        yield client
    except OSError:
        client.abort()
        raise
    finally:
        client.close()
