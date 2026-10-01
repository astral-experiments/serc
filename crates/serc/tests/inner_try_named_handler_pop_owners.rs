#[allow(dead_code)]
#[path = "common/optimization.rs"]
mod cpython;

use cpython::{LEVELS, assert_matches_cpython};

#[test]
fn saved_inner_try_returns_preserve_complete_named_handler_pop_cells() {
    for (filename, source) in [
        (
            "guarded.py",
            r"async def f(flag, value, action, consume):
    try:
        try:
            await action()
        except Exception as error:
            try:
                if flag:
                    return action()
            except TypeError:
                await consume(value)
    finally:
        if value is not None:
            await consume(value)
",
        ),
        (
            "unguarded.py",
            r"async def f(flag, value, action, consume):
    try:
        try:
            await action()
        except Exception as error:
            try:
                return action()
            except TypeError:
                await consume(value)
    finally:
        if value is not None:
            await consume(value)
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}
