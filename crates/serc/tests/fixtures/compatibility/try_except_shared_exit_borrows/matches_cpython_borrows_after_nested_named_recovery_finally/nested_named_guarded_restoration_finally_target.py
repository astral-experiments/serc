def publish(resource, work, acquire, restore, report, enabled):
    try:
        report(resource)
        try:
            work(resource, mode=enabled or resource)
            completed = True
            report(completed)
        except Exception as error:
            code = report(error)
            if code != 0 and not enabled:
                raise
            report(error)
            saved = restore(resource)
            if saved:
                raise
            try:
                restore(resource)
                resource = None
                retry = acquire()
                try:
                    work(retry)
                    completed = True
                    report(completed)
                finally:
                    retry.close()
            finally:
                restore(saved)
    finally:
        if resource:
            resource.close()
