def probe(work, error):
    try:
        local_error = work()
    except KeyError:
        pass
    except (local_error, error):
        pass
