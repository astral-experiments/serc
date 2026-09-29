def write_batches(batches, opener, report, stopped, delay, completed):
    index = 0
    while index < len(batches):
        items = batches[index]
        index += 1
        while True:
            try:
                with opener() as sink:
                    for item in items:
                        sink.write(item)
                break
            except Exception as error:
                report(error)
                if stopped():
                    break
                delay()
        completed()
