def run(get, stop, process, recover, report, done, publish, close):
    while True:
        item = get()
        result = None
        try:
            if item is stop:
                break
            key, value = item
            result = process(key, value)
        except Exception as error:
            report(error)
            if isinstance(item, tuple) and len(item) == 2 and isinstance(item[0], int):
                result = recover(item[0])
        finally:
            done()
        if result is not None:
            publish(result)
