use crate::common;

#[test]
fn shared_original_aliases_do_not_retain_cleanup_jumps() {
    common::assert_matches_cpython_marshal(
        r"def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    with manager:
                        call()
        except Exception:
            call()
        finally:
            finish()
",
        "with_nested_if-sync-2.py",
    );
    common::assert_matches_cpython_marshal(
        r"def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    if a is not None:
                        with manager:
                            call()
        except Exception:
            call()
        finally:
            finish()
",
        "with_nested_if-sync-3.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    with manager:
                        await call()
        except Exception:
            await call()
        finally:
            await finish()
",
        "with_nested_if-async-2.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def sample(a, b, manager, call, finish):
    while a:
        try:
            if a:
                if b:
                    if a is not None:
                        with manager:
                            await call()
        except Exception:
            await call()
        finally:
            await finish()
",
        "with_nested_if-async-3.py",
    );
    common::assert_matches_cpython_marshal(
        r"def sample(a, b, call, finish):
    try:
        if a:
            if b:
                try:
                    call()
                except Exception:
                    call()
    except BaseException:
        call()
    finally:
        finish()
",
        "normal_try_nested_if-sync-2.py",
    );
    common::assert_matches_cpython_marshal(
        r"def sample(a, b, call, finish):
    try:
        if a:
            if b:
                if a is not None:
                    try:
                        call()
                    except Exception:
                        call()
    except BaseException:
        call()
    finally:
        finish()
",
        "normal_try_nested_if-sync-3.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def sample(a, b, call, finish):
    try:
        if a:
            if b:
                try:
                    await call()
                except Exception:
                    await call()
    except BaseException:
        await call()
    finally:
        await finish()
",
        "normal_try_nested_if-async-2.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def sample(a, b, call, finish):
    try:
        if a:
            if b:
                if a is not None:
                    try:
                        await call()
                    except Exception:
                        await call()
    except BaseException:
        await call()
    finally:
        await finish()
",
        "normal_try_nested_if-async-3.py",
    );
}
