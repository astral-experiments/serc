def build(worker, report, kind):
    source = "direct"
    try:
        result = worker.build(kind)
    except Exception as error:
        report(error)
        source = "fallback"
        try:
            spec = worker.retry(kind)
        except Exception as inner:
            report(inner)
            saved = worker.method
            worker.method = lambda value: -1
            spec = worker.retry(kind)
            worker.method = saved
        result = {"spec": spec, "source": source, "tokens": list(worker.tokens())}
    return result, source, kind
