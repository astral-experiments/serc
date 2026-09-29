def name_is_none_return(manager, old_span):
    with manager:
        if old_span is None:
            return old_span
    return 1


def not_method_call_return(manager, client):
    with manager:
        if not client.request():
            return None
    return 1


def not_call_args_return(manager, command, install_args):
    with manager:
        if not command(setup, install, *install_args):
            return None
    return 1


def name_return(manager, cache_checked):
    with manager:
        if cache_checked:
            return cache_checked
    return None


def name_collection_return(manager, file_list):
    with manager:
        if file_list:
            return file_list
    return None


def not_name_raise(manager, unlock_pool):
    with manager:
        if not unlock_pool:
            raise RuntimeError(unlock_pool)
    return None


def name_is_not_none_return(manager, trial):
    with manager:
        if trial is not None:
            return trial
    return None


def not_attribute_raise(manager, cursor):
    with manager:
        if not cursor.rowcount:
            raise RuntimeError(cursor)
    return None


def named_call_return(manager, process):
    with manager:
        if pid := process.fork():
            return pid
    return None


def not_name_raise_body(manager, grammar):
    with manager:
        if not grammar:
            observe(grammar)
            raise RuntimeError(grammar)
    return None


def not_call_many_args_return(manager, checker, callee, types, kinds, names, mapping):
    with manager:
        if not checker.check(callee, types, kinds, names, mapping, None):
            return None
    return 1


def named_static_call_return(manager, provider, name):
    with manager:
        if python := provider.find(name):
            return python
    return None


def named_keyword_call_return(manager, lookup, cursor, cache_type, label):
    with manager:
        if found_name := lookup(cursor=cursor, key_parts=(cache_type, label)):
            return found_name
    return None


def named_is_none_raise(manager, cursor):
    with manager:
        try:
            cursor.execute()
        except RuntimeError as error:
            raise ValueError(error) from error
        if (identifier_row := cursor.fetchone()) is None:
            raise RuntimeError(identifier_row)
    return identifier_row


def named_keyword_is_none_return(manager, database, cursor, name, tiers):
    with manager:
        if (cached_value := database.get(cursor=cursor, name=name)) is None:
            return None
    for tier in tiers:
        if tier.name == cached_value:
            return tier
    return None


def named_keyword_is_none_return_body(manager, database, cursor, name):
    with manager:
        if (result := database.get(cursor=cursor, name=name)) is None:
            return None
    try:
        raw_data = database.decode(result)
    except ValueError:
        return None
    try:
        return database.load(raw_data)
    except KeyError:
        return None


def not_name_lines_raise(manager, lines):
    with manager:
        if not lines:
            raise RuntimeError(lines)
    return None


def not_name_grammar_raise(manager, grammar):
    with manager:
        if not grammar:
            raise RuntimeError(grammar)
    return None


def prior_call_comprehension_return(manager, condition, items):
    result = consume([item for item in items])
    with manager:
        if condition:
            return result


def earlier_call_comprehension_return(manager, condition, items):
    consume([item for item in items])
    prepare()
    with manager:
        if condition:
            return condition
