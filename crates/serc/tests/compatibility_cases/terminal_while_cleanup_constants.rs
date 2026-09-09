use crate::common;

#[test]
fn terminal_while_cleanup_constants_preserve_nested_scope_barriers() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with factory(lambda x='default': x):
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with factory(x for x in ('iterable',)):
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager, factory(x for x in ('iterable',)):
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            callback = lambda x='default': x
            if stop:
                break
            consume('later')
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_optional_targets() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as slots['target']:
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as slots[:bound]:
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as (value, slots['target']):
            if stop:
                break
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as first, factory(value) as slots['target']:
            if stop:
                break
            consume('later')
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_function_and_coroutine_finalizers() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    break
            finally:
                consume('cleanup')
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    continue
            finally:
                consume('cleanup')
            consume('later')
            if stop:
                break
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    break
            finally:
                consume('cleanup')
            consume('later')
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    continue
            finally:
                consume('cleanup')
            consume('later')
            if stop:
                break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_generator_finalizers() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            try:
                if stop:
                    break
            finally:
                consume('cleanup')
            consume('later')
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            try:
                if stop:
                    continue
            finally:
                consume('cleanup')
            consume('later')
            if stop:
                break
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            try:
                if stop:
                    break
            finally:
                consume('cleanup')
            consume('later')
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            try:
                if stop:
                    continue
            finally:
                consume('cleanup')
            consume('later')
            if stop:
                break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_function_and_coroutine_returns() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            if stop:
                return 'answer'
            consume('later')
            if retry:
                break
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    return value
            finally:
                consume('cleanup')
            consume('later')
            if retry:
                break
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            if stop:
                return 'answer'
            consume('later')
            if retry:
                break
",
        r"async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    return value
            finally:
                consume('cleanup')
            consume('later')
            if retry:
                break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_generator_and_compound_returns() {
    for source in [
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            if stop:
                return 'answer'
            consume('later')
            if retry:
                break
",
        r"def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    yield value
    while stop:
        with manager:
            try:
                if stop:
                    return value
            finally:
                consume('cleanup')
            consume('later')
            if retry:
                break
",
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return ('left', 'right')
            consume(value)
        consume('later')
        if retry:
            break
",
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return -7
            consume(value)
        consume('later')
        if retry:
            break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_compound_and_effectful_returns() {
    for source in [
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return 'an' + 'swer'
            consume(value)
        consume('later')
        if retry:
            break
",
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return factory('effect')
            consume(value)
        consume('later')
        if retry:
            break
",
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        try:
            with manager:
                if stop:
                    return factory('effect')
                consume(value)
        finally:
            consume('cleanup')
        consume('later')
        if retry:
            break
",
        r"def probe(manager, factory, value, stop, retry):
    while stop:
        try:
            with manager:
                if stop:
                    return value
                consume(value)
        finally:
            consume('cleanup')
        consume('later')
        if retry:
            break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}

#[test]
fn terminal_while_cleanup_constants_preserve_else_before_handler_cleanup() {
    for source in [
        r"def probe(manager, value, stop, retry):
    while stop:
        with manager:
            try:
                consume(value)
            except Error:
                if retry:
                    break
            else:
                consume('else-first')
            if stop:
                break
",
        r"def probe(manager, value, stop, retry):
    while stop:
        with manager:
            try:
                consume(value)
            except Error:
                if retry:
                    continue
            else:
                consume('else-first')
            if stop:
                break
",
        r"def probe(manager, value, stop, retry):
    while stop:
        with manager:
            try:
                consume(value)
            except Error as caught:
                if retry:
                    break
            else:
                consume('else-first')
            if stop:
                break
",
        r"def probe(manager, value, stop, retry):
    while stop:
        with manager:
            try:
                consume(value)
            except Error as caught:
                if retry:
                    continue
            else:
                consume('else-first')
            if stop:
                break
",
    ] {
        common::assert_matches_cpython_marshal(source, "terminal_while_cleanup_constants.py");
    }
}
