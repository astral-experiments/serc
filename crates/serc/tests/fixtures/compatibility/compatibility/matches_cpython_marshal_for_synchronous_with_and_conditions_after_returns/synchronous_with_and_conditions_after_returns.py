def and_name_call(manager, cached, flag, fresh):
    with manager:
        if cached is None:
            return None
        if flag and fresh(cached):
            return cached
        return None


def and_attribute_compare(manager, cached, state, touch):
    with manager:
        if cached is None:
            return None
        if state.ready and cached != state:
            touch(cached)
        return cached


def and_not_name_attribute(manager, cached, flag, state, touch):
    with manager:
        if cached is None:
            return None
        if not flag and state.ready:
            touch(cached)
        return cached


def and_not_attribute_call(manager, cached, state, fresh):
    with manager:
        if cached is None:
            return None
        if not state.ready and fresh(cached):
            return cached
        return None


def and_three_operands(manager, cached, flag, state, fresh):
    with manager:
        if cached is None:
            return None
        if flag and state.ready and fresh(cached):
            return cached
        return None


def compare_and_name_control(manager, cached, flag, state, touch):
    with manager:
        if cached is None:
            return None
        if cached != state and flag:
            touch(cached)
        return cached


def nested_nonterminal_return_control(manager, cached, flag, state, touch):
    with manager:
        if cached is None:
            if flag:
                return None
            touch(cached)
        if flag and state.ready:
            touch(cached)
        return cached


def prior_raise_control(manager, cached, flag, state, touch):
    with manager:
        if cached is None:
            raise ValueError
        if flag and state.ready:
            touch(cached)
        return cached


def loop_control(manager, values, flag, state, touch):
    with manager:
        for cached in values:
            if cached is None:
                return None
            if flag and state.ready:
                touch(cached)
        return None
