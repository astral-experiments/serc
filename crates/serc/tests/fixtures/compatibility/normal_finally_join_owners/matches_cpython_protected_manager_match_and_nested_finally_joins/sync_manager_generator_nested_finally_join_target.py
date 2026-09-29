def nested_cleanup(manager, resource, lease):
    with manager:
        try:
            yield
        finally:
            try:
                restore()
            finally:
                try:
                    if resource is not None:
                        resource.close()
                finally:
                    if lease is not None:
                        try:
                            unlock(lease)
                        finally:
                            close(lease)
