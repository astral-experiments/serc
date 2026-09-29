def recover(fetch, pause, attempts, limit, wanted, error):
    for attempt in attempts:
        try:
            observed = fetch()
        except Exception as failure:
            if attempt + 1 == limit:
                raise error from failure
        else:
            if observed is not None and observed.key == wanted and observed.ready:
                return observed
        if attempt + 1 < limit:
            pause()
    raise error
