use crate::common;

#[test]
fn async_generator_handler_inner_try_return() {
    common::assert_matches_cpython_marshal(
        r"async def f(values):
    try:
        try:
            first()
        except FirstError:
            try:
                async for value in values:
                    yield value
                    if value:
                        return
            except SecondError as error:
                raise FinalError from error
    except OuterError:
        raise
",
        "async_generator_handler_inner_try_return.py",
    );
}

#[test]
fn async_generator_handler_no_outer_try_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(values):
    try:
        first()
    except FirstError:
        try:
            async for value in values:
                yield value
                if value:
                    return
        except SecondError as error:
            raise FinalError from error
",
        "async_generator_handler_no_outer_try_control.py",
    );
}

#[test]
fn async_generator_handler_no_inner_try_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(values):
    try:
        try:
            first()
        except FirstError:
            async for value in values:
                yield value
                if value:
                    return
    except OuterError:
        raise
",
        "async_generator_handler_no_inner_try_control.py",
    );
}
