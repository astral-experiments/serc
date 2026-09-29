def list_fallback(manager, lookup):
    with manager:
        payload = {
            "left": len(lookup("left") or []),
            "right": len(lookup("right") or []),
        }
    return payload


def nonempty_list_fallback(manager, lookup):
    with manager:
        payload = {
            "left": len(lookup("left") and [None]),
            "right": len(lookup("right") and [None]),
            "extra": len(lookup("extra") and [None]),
        }
    return payload


def tuple_fallback(manager, lookup):
    with manager:
        payload = (
            len(lookup("left") or ()),
            len(lookup("right") or ()),
        )
    return payload


def dictionary_fallback(manager, lookup):
    with manager:
        payload = {
            "left": len(lookup("left") and {}),
            "right": len(lookup("right") and {}),
        }
    return payload


def set_fallback(manager, lookup):
    with manager:
        payload = (
            len(lookup("left") or {None}),
            len(lookup("right") or {None}),
        )
    return payload


def name_fallback(manager, lookup, fallback):
    with manager:
        payload = {
            "left": len(lookup("left") or fallback),
            "right": len(lookup("right") or fallback),
        }
    return payload


def attribute_fallback(manager, lookup, options):
    with manager:
        payload = (
            len(lookup("left") or options.default),
            len(lookup("right") or options.default),
        )
    return payload


def keyword_lookup(manager, lookup):
    with manager:
        payload = {
            "left": len(lookup(name="left") or []),
            "right": len(lookup(name="right") or []),
        }
    return payload


def starred_lookup(manager, lookup, values):
    with manager:
        payload = (
            len(lookup(*values) and {}),
            len(lookup(*values) and {}),
        )
    return payload


def unpacked_lookup(manager, lookup, mapping):
    with manager:
        payload = (
            len(lookup(**mapping) or ()),
            len(lookup(**mapping) or ()),
        )
    return payload


def preceding_action(manager, lookup, consume):
    with manager:
        consume("before")
        payload = (
            len(lookup("left") or []),
            len(lookup("right") or []),
            len(lookup("extra") or []),
        )
    return payload


def sibling_context(first, second, lookup, consume):
    with first:
        consume(lookup("earlier") or [])
    with second:
        payload = (
            len(lookup("left") or []),
            len(lookup("right") or []),
        )
    return payload


def direct_marker_then_wrapped(manager, lookup, consume):
    with manager:
        consume(lookup("earlier") or [])
        payload = len(lookup("later") and [])
    return payload


def direct_boolean_control(manager, lookup):
    with manager:
        payload = {
            "left": lookup("left") or [],
            "right": lookup("right") or [],
        }
    return payload


async def coroutine_control(manager, lookup, consume):
    with manager:
        payload = (
            consume(lookup("left") or []),
            consume(lookup("right") or []),
        )
    return payload


def generator_control(manager, lookup, consume):
    with manager:
        payload = (
            consume(lookup("left") or []),
            consume(lookup("right") or []),
        )
    yield payload


async def async_with_control(manager, lookup, consume):
    async with manager:
        payload = (
            consume(lookup("left") or []),
            consume(lookup("right") or []),
        )
    return payload
