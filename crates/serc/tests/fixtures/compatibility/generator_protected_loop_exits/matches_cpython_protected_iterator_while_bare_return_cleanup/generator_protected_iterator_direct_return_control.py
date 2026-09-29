def run(items, ready, finish):
    try:
        for item in items:
            yield item
            if ready(item):
                finish()
                return
    except BaseException:
        finish()
        raise
