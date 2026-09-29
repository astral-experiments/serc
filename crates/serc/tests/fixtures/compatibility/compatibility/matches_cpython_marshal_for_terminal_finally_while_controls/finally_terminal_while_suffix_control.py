def run(service, release, clock):
    try:
        start(service)
    finally:
        release()
        deadline = clock() + 2
        while service.pending and clock() < deadline:
            pause()
        complete()
    assert service.pending == {}
