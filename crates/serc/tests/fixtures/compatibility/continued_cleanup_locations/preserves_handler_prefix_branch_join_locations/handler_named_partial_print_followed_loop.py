def f(items, call, enabled, details, save):
    for item in items:
        try:
            call(item)
        except Exception as exc:
            save(exc)
            if enabled:
                if details:
                    save(item)
                else:
                    save(None)
                save(exc)
        save(item)
