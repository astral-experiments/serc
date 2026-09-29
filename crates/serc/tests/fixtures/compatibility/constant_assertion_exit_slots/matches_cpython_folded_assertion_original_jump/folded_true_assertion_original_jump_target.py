def validate(items):
    for entry in items:
        if entry.enabled:
            assert entry.value is None if not entry.selected else True, "invalid value"
        else:
            assert entry.value is None
        consume(entry)
