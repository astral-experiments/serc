use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_direct_none_condition_owners() {
    let source = r##"def deferred_instrumentation(cls, value, record):
    if not record(value):
        return
    with cls.lock:
        if cls.instance is None:
            cls.pending.append(value)
            return
    record(value)


def parse_owners(manager, lines, path_to_regex, append):
    with manager:
        for line in lines:
            line = line.strip()
            if "#" in line:
                line = line.split("#", 1)[0].strip()
            if line == "":
                continue
            if line.startswith("[") and line.endswith("]"):
                continue
            elements = line.split()
            if len(elements) < 2:
                continue
            path = elements[0]
            if path is None:
                continue
            try:
                pattern = path_to_regex(path)
            except (ValueError, IndexError):
                continue
            owners = [owner for owner in elements[1:] if owner]
            if not owners:
                continue
            append((pattern, owners))


def nested_complete_else(manager, kwargs, expected, func, a, b):
    with manager:
        expected_a, expected_b = expected
        expected_b = expected_b or b
        assert func(a, b=b) == (a, b)
        if "a" in kwargs:
            if expected_a is not None:
                assert func() == (expected_a, expected_b)
                assert func(b=b) == (expected_a, b)
            else:
                func()
        else:
            func()


def terminal_raise(custom_asset):
    with GlobalDBHandler().conn.read_ctx() as cursor:
        result = cursor.execute(
            "SELECT value FROM entries WHERE type=? AND name=? AND identifier!=?",
            (
                custom_asset.custom_asset_type,
                custom_asset.name,
                custom_asset.identifier,
            ),
        ).fetchone()
        if result is not None:
            raise InputError(
                f'entry with name "{custom_asset.name}" and type '
                f'"{custom_asset.custom_asset_type}" already exists'
            )


def walrus_return(node):
    with GlobalDBHandler().conn.read_ctx() as cursor:
        if (value := globaldb_get_unique_cache_value(
            cursor=cursor,
            key_parts=(CacheType.NAMEHASH, node.hex()),
        )) is not None:
            return value
    return None


def visit_filter(self, node, frame):
    with self._filter_test_common(node, frame, True):
        if node.node is not None:
            self.visit(node.node, frame)
        elif frame.eval_ctx.volatile:
            self.write(f"concat({frame.buffer})")
        elif frame.eval_ctx.autoescape:
            self.write(f"Markup(concat({frame.buffer}))")
        else:
            self.write(f"concat({frame.buffer})")


def single_statement_control(manager, value, record, after):
    with manager:
        if value is not None:
            record(value)
        after()


def while_break_control(manager, values, record):
    with manager:
        while values:
            value = values.pop()
            if value is None:
                break
            record(value)


def complete_else_control(manager, value, record, other, after):
    with manager:
        if value is not None:
            record(value)
        else:
            other(value)
        after()


def no_with_control(value, record, finish):
    if value is not None:
        record(value)
        finish(value)


def reversed_control(manager, value, record, finish):
    with manager:
        if None is not value:
            record(value)
            finish(value)


def equality_control(manager, value, record, finish):
    with manager:
        if value == None:
            record(value)
            finish(value)


def chained_control(manager, value, other, record, finish):
    with manager:
        if value is not None is not other:
            record(value)
            finish(value)


async def async_with_control(manager, value, record, finish):
    async with manager:
        if value is not None:
            record(value)
            finish(value)
"##;
    common::assert_matches_cpython_marshal(
        source,
        "synchronous_with_direct_none_condition_owner_compatibility.py",
    );
}

#[test]
fn terminal_with_folded_none_condition_owners_match_cpython_marshal() {
    let source = r"async def f(manager, work, write):
    with manager:
        value = await work()
        if value is not None:
            return value
    return await write()

async def g(manager, work, write):
    with manager:
        value = await work()
        if value:
            return value
    return await write()

def h(manager, work, write):
    with manager:
        value = work()
        if value is not None:
            return value
    return write()
";

    common::assert_matches_cpython_marshal(source, "terminal_with_folded_none_condition_owners.py");
}
