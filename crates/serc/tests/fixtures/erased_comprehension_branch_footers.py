def list_branches(flag, items):
    if flag:
        out = [item for item in items]
        assert out
    else:
        out = [item for item in items]
        assert out
    assert flag


async def async_list_branches(flag, items):
    if flag:
        out = [item for item in items]
        assert out
    else:
        out = [item for item in items]
        assert out
    assert flag


def set_branches(flag, items):
    if flag:
        consume(items)
        out = {item.value for item in items if item.active}
        assert out
    else:
        consume(items)
        out = {item.value for item in items if item.active}
        assert out
    assert flag


async def async_dict_branches(flag, items):
    if flag:
        value = consume(items)
        out = {item: item[0] for item in items}
        assert out
        assert value
    else:
        value = consume(items)
        out = {item: item[0] for item in items}
        assert out
        assert value
    assert flag


def call_sibling(flag, items):
    if flag:
        consume(items)
    else:
        out = [item for item in items]
        assert out
    assert flag


async def async_name_sibling(flag, items):
    if flag:
        out = items
    else:
        out = [item for item in items]
        assert out
    assert flag


def no_branch_assertion(flag, items):
    if flag:
        out = [item for item in items]
    else:
        out = [item for item in items]
    assert flag


async def no_outer_assertion(flag, items):
    if flag:
        out = [item for item in items]
        assert out
    else:
        out = [item for item in items]
        assert out


def live_suffix(flag, items):
    if flag:
        out = [item for item in items]
        consume(out)
    else:
        out = [item for item in items]
        consume(out)
    assert flag


def comprehension_prefixes(flag, items):
    if flag:
        prefix = [item for item in items]
        out = [item for item in prefix]
        assert out
    else:
        prefix = [item for group in items for item in group]
        out = [item for item in prefix]
        assert out
    assert flag


async def mixed_comprehension_prefixes(flag, items):
    if flag:
        prefix = {item for group in items if group for item in group if item}
        out = {item: item.value for item in prefix}
        assert out
    else:
        prefix = {item: item.value for item in items}
        out = {item for item in prefix}
        assert out
    assert flag


def one_comprehension_prefix(flag, items):
    if flag:
        out = [item for item in items]
        assert out
    else:
        prefix = [item for item in items]
        out = [item for item in prefix]
        assert out
    assert flag
