def run(items, ready, stopped, finish):
    try:
        for item in items:
            while True:
                yield item
                if ready(item):
                    finish()
                    return
                if stopped(item):
                    finish()
                    return
                break
    except BaseException:
        finish()
        raise
