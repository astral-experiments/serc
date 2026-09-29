def sync_context(manager, items):
    for item in items:
        with manager:
            if item > 0:
                continue
            break

def exception_handler(items):
    for item in items:
        try:
            consume(item)
        except Exception:
            handle(item)
            continue
        use(item)

try:
    for item in items:
        consume(item)
        break
except TypeError:
    handle()
after()
