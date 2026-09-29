import service

def f(records, directory, options, submitted, counters):
    pending = {}
    try:
        for record in records:
            pending[record] = submit(record)
        while pending:
            for ref in drain(pending):
                handle(ref)
            report(pending)
        report_status(directory=directory, options=options, status="finished",
                      submitted=submitted, pending=0, counters=counters)
        report_progress(options=options, submitted=submitted, pending=0, counters=counters)
        report()
        report(
            "Finished work. "
            f"Wrote {len(good(directory))} results and "
            f"{len(bad(directory))} failures under {directory}."
        )
        return 0
    except KeyboardInterrupt:
        for ref in pending:
            service.cancel(ref, force=True)
        if options is not None:
            report(options)
        return 130
    except Exception as error:
        if options is not None:
            report(error)
        raise
    finally:
        if service.is_initialized():
            service.shutdown()
