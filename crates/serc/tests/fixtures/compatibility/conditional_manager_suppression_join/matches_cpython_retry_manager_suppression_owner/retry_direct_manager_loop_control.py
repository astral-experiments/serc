def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            with open_remote(source) as handle:
                return handle.read()
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
