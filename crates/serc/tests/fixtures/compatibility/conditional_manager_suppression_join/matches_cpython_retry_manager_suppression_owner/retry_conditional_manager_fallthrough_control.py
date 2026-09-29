def read_source(source, retries):
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                with open_remote(source) as handle:
                    consume(handle.read())
            return read_local(source)
        except Exception as error:
            if attempt >= retries:
                raise
            pause(attempt)
    raise RuntimeError()
