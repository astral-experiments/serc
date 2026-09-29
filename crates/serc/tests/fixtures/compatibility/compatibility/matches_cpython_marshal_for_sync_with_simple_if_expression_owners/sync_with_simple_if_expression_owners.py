def assignment_name(manager, enabled, first):
    with manager:
        if enabled:
            return first
        result = "left" if first else "right"
        observe(result)


def assignment_attribute(manager, enabled, holder):
    with manager:
        if enabled:
            return holder
        result = "left" if holder.first else "right"
        observe(result)


async def coroutine_attribute(manager, enabled, holder):
    with manager:
        if enabled:
            return holder
        result = "left" if holder.first.second else "right"
        observe(result)


def call_test_is_protected(manager, enabled, first):
    with manager:
        if enabled:
            return first
        result = "left" if prepare(first) else "right"
        observe(result)


def complex_attribute_root_is_protected(manager, enabled, first):
    with manager:
        if enabled:
            return first
        result = "left" if prepare(first).value else "right"
        observe(result)


def call_argument_is_protected(manager, enabled, first):
    with manager:
        if enabled:
            return first
        observe("left" if first else "right")


def right_operand_is_protected(manager, enabled, first):
    with manager:
        if enabled:
            return first
        result = first + ("left" if first else "right")
        observe(result)


def later_collection_element_is_protected(manager, enabled, first):
    with manager:
        if enabled:
            return first
        result = [first, "left" if first else "right"]
        observe(result)


def assignment_gap_is_unchanged(manager, enabled, first, second):
    with manager:
        if enabled:
            return first
        prepared = prepare(first)
        result = prepared if first and not second else first
        observe(result)


def nonterminal_condition_is_unchanged(manager, enabled, first):
    with manager:
        if enabled:
            pass
        result = "left" if first else "right"
        observe(result)
