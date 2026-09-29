def drain(manager, stop, tick, report):
    resource = manager()
    try:
        while True:
            delay = tick()
            if stop.wait(delay):
                break
            health = tick()
            report(health)
            if health.ready:
                report(health.value)
    finally:
        report.info("closing", extra={"address": resource.address})
    report(None)
