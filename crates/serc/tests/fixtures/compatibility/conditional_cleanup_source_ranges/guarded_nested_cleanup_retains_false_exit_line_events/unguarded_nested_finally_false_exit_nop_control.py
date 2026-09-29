def run(resource, ready, work, emit):
    try:
        work()
    finally:
        try:
            if ready:
                work()
        finally:
            resource.cleanup()
    emit(resource)
