def f(sink, item, retry):
    while ready():
        try:
            work(item)
        except Error as error:
            record(error)
            if retry:
                reschedule(item)
                break
            failed(item)
        finally:
            sink.record(item)
            sink.done()
    after(sink)
