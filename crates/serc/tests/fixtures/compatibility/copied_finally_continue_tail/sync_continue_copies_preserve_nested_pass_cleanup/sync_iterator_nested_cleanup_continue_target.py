def fetch_retry(attempts, open_resource, fetch, exists, remove, retry):
    for attempt in attempts:
        path = None
        try:
            with open_resource() as resource:
                path = resource.name
            return fetch(path), None
        except Exception:
            if retry(attempt):
                continue
            return None, None
        finally:
            if path and exists(path):
                try:
                    remove(path)
                except Exception:
                    pass
