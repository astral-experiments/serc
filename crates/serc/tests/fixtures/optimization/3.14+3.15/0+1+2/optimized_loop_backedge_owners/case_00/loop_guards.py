def check_flags(items, manager, enabled, disabled):
    try:
        with manager:
            for item in items:
                if enabled and not disabled:
                    assert admitted(item)
                else:
                    assert denied(item)
    finally:
        cleanup()


def collect(groups, manager):
    with manager:
        for enabled, values in groups:
            if enabled:
                for value in values:
                    assert value is not None
                    record(value)
            else:
                assert not values


def validate_options(items, manager, enabled):
    with manager:
        for item in items:
            if enabled:
                pass
            else:
                if item.valid:
                    assert accepts(item)
                assert item.default


def check_names(names, cache):
    try:
        for name in names:
            if name.startswith("a.") or name in cache:
                assert cache[name]
    finally:
        cleanup()


def skip_known(items, enabled, known):
    for item in items:
        if enabled:
            if item in known:
                value = known[item]
                if value:
                    assert accepts(value)
                continue
            assert valid(item)
