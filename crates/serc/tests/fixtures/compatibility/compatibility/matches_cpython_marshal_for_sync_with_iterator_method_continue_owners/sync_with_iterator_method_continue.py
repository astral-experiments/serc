def prior_assignment(manager, values, make, use):
    with manager:
        for value in values:
            current = make(value)
            if not current.ready():
                continue
            use(current)


def prior_call(manager, values, observe, use):
    with manager:
        for value in values:
            observe(value)
            if not value.ready():
                continue
            use(value)


def nested_for(manager, groups, values, make, use):
    with manager:
        for group in groups:
            for value in values:
                current = make(value)
                if not current.ready():
                    continue
                use(group, current)


def nested_while(manager, groups, values, use):
    with manager:
        while groups:
            for value in values:
                if not value.ready():
                    continue
                use(value)
            groups.pop()


def prior_loop_control(manager, groups, values, use):
    with manager:
        for value in values:
            for group in groups:
                use(group)
            if not value.ready():
                continue
            use(value)


def prior_nested_loop_control(manager, groups, others, values, use):
    with manager:
        for value in values:
            for group in groups:
                for other in others:
                    use(group, other)
            if not value.ready():
                continue
            use(value)


def prior_marker_control(manager, values, use, flag):
    with manager:
        for value in values:
            if flag:
                use(value)
            if not value.ready():
                continue
            use(value)


def body_control(manager, values, observe, use):
    with manager:
        for value in values:
            if not value.ready():
                observe(value)
            use(value)


def break_control(manager, values, use):
    with manager:
        for value in values:
            if not value.ready():
                break
            use(value)


async def async_with_control(manager, values, use):
    async with manager:
        for value in values:
            if not value.ready():
                continue
            use(value)


def try_control(manager, values, use):
    try:
        with manager:
            for value in values:
                if not value.ready():
                    continue
                use(value)
    except Error:
        use(None)
