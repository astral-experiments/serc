def run(resource, ready, work, emit):
    try:
        work()
    finally:
        if resource is not None:
            try:
                if ready:
                    work()
            finally:
                resource.cleanup()
    emit(resource)
