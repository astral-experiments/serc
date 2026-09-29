def fetch_parsed(attempts, fetch, decode, pause):
    last_error = None
    for attempt in attempts:
        try:
            payload = fetch()
            try:
                return decode(payload) if payload else {}
            except ValueError as error:
                raise RuntimeError('decode failed') from error
        except Exception as error:
            last_error = error
            pause(attempt)
    raise RuntimeError('attempts exhausted') from last_error
