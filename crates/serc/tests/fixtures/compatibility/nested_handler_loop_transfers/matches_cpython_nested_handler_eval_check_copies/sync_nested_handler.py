def f(stop, wait, heartbeat, sink):
    while not stop.is_set():
        try:
            wait(stop)
        except TimeoutError:
            try:
                heartbeat()
                if sink is not None:
                    sink.write()
            except Exception as exc:
                report(exc)
