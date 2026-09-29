def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                with open_remote(source) as handle:
                    return handle.read()
                break
            return read_local(source)
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
