use crate::common::assert_matches_cpython_marshal;

#[test]
fn original_boolean_prefixes_preserve_the_donor_region() {
    for (filename, source) in [
        (
            "coroutine_old_prefix_no_fusion.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        x8 = seed()
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
        (
            "coroutine_old_prefix_one_fusion.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        y0 = pair(a, b)
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
        (
            "coroutine_old_prefix_two_fusions.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        y0 = pair(a, b)
        y1 = pair(a, b)
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
        (
            "coroutine_latest_fusion_control.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        y0 = pair(a, b)
        y1 = pair(a, b)
        y2 = pair(a, b)
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
        (
            "coroutine_outer_try_prefix.py",
            r"async def f(values, a, b, flag, receiver):
    try:
        for item in values:
            x0 = seed()
            x1 = seed()
            x2 = seed()
            x3 = seed()
            x4 = seed()
            x5 = seed()
            x6 = seed()
            x7 = seed()
            y0 = pair(a, b)
            stamp = clock.now()
            try:
                async for value in receiver.stream(
                    a,
                    flag or DEFAULT,
                ):
                    await consume(value)
            except Error:
                recover()
    except Outer:
        recover_outer()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn boolean_prefix_owners_respect_null_slots_and_source_joins() {
    for (filename, source) in [
        (
            "synchronous_null_prefix_control.py",
            r"def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        y0 = pair(a, b)
        stamp = clock.now()
        try:
            receiver.stream(
                a,
                flag or DEFAULT,
            )
        except Error:
            recover()
",
        ),
        (
            "coroutine_fresh_append_control.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
        (
            "coroutine_source_join_control.py",
            r"async def f(values, a, b, flag, receiver):
    for item in values:
        x0 = seed()
        x1 = seed()
        x2 = seed()
        x3 = seed()
        x4 = seed()
        x5 = seed()
        x6 = seed()
        x7 = seed()
        y0 = pair(a, b)
        if item:
            receiver.flush()
        stamp = clock.now()
        try:
            async for value in receiver.stream(
                a,
                flag or DEFAULT,
            ):
                await consume(value)
        except Error:
            recover()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
