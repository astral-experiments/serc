def f(items, callback):
    for recipient in items:
        if recipient.enabled:
            callback(TOKEN)
            try:
                if recipient.name.startswith("x"):
                    consume(recipient)
            except Exception:
                recover()
        elif recipient.other:
            callback(OTHER)
