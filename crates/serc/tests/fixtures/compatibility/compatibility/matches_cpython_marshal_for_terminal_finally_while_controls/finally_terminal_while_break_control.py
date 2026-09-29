def run(service, release, clock):
    try:
        start(service)
    finally:
        release()
        deadline = clock() + 2
        while service.pending and clock() < deadline:
            if stop():
                break
            pause()
    assert service.pending == {}
