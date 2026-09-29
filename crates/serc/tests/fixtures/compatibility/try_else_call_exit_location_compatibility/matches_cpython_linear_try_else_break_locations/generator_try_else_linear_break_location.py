def run(client, attempts, release):
    try:
        for attempt in attempts:
            try:
                ready = client.ping()
            except Exception:
                report(attempt)
            else:
                if ready:
                    report(ready)
                    break
            pause(attempt)
        yield client
    finally:
        release()
