def read_source(source, retries):
    last_error = None
    for attempt in range(retries + 1):
        try:
            if is_remote(source):
                request = prepare(source)
                with open_remote(request) as handle:
                    return handle.read()
            if is_file(source):
                return read_file(source)
            return read_local(source)
        except Exception as error:
            last_error = error
            if attempt >= retries:
                break
            pause(attempt)
    raise last_error
