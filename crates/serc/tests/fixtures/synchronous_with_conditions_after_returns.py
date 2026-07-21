def none_return_then_not(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        if not flag:
            touch(cached)
        return cached


def none_return_then_name(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        if flag:
            touch(cached)
        return cached


def none_return_then_attribute(manager, cached, touch):
    with manager:
        if cached is None:
            return None
        if cached.ready:
            touch(cached)
        return cached


def name_return_then_not(manager, cached, flag, touch):
    with manager:
        if cached:
            return cached
        if not flag:
            touch(cached)
        return None


def call_return_then_not(manager, cached, flag, touch):
    with manager:
        if touch(cached):
            return cached
        if not flag:
            touch(cached)
        return None


def compare_return_then_not(manager, cached, flag, touch):
    with manager:
        if cached == flag:
            return cached
        if not flag:
            touch(cached)
        return None


def bare_return_then_not(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return
        if not flag:
            touch(cached)
        return cached


def work_return_then_attribute(manager, cached, flag, touch):
    with manager:
        if cached is None:
            touch(flag)
            return None
        if flag.ready:
            touch(cached)
        return cached


def nested_try_return_then_not(manager, cached, flag, touch):
    try:
        with manager:
            if cached is None:
                return None
            if not flag:
                touch(cached)
            return cached
    except ValueError:
        return None


def nested_manager_return_then_not(first, second, cached, flag, touch):
    with first:
        with second:
            if cached is None:
                return None
            if not flag:
                touch(cached)
            return cached


def compare_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        if cached == flag:
            touch(cached)
        return cached


def none_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        if flag is not None:
            touch(cached)
        return cached


def subscript_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        if cached[flag]:
            touch(cached)
        return cached


def raise_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            raise ValueError
        if not flag:
            touch(cached)
        return cached


def nested_nonterminal_return_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            if flag:
                return None
            touch(cached)
        if not flag:
            touch(cached)
        return cached


def elif_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            return None
        elif cached == flag:
            touch(cached)
        if flag:
            touch(cached)
        return cached


def no_return_control(manager, cached, flag, touch):
    with manager:
        if cached is None:
            touch(cached)
        if not flag:
            touch(cached)
        return cached


def no_prior_control(manager, cached, flag, touch):
    with manager:
        if not flag:
            touch(cached)
        return cached


def loop_control(manager, values, flag, touch):
    with manager:
        for cached in values:
            if cached is None:
                return None
            if not flag:
                touch(cached)
        return None
