use crate::common;

#[test]
fn ordinary_named_return_preserves_finally_condition_owners() {
    common::assert_matches_cpython_marshal(
        r#"async def f():
    client = None
    try:
        plan = await desired()
        client = await connect()
        handle = client.handle(plan)
        try:
            await handle.describe()
        except RPCError as error:
            if error.status != NOT_FOUND:
                raise
            try:
                await client.create(plan)
                log("created")
                return
            except AlreadyRunning:
                handle = client.handle(plan)
        await handle.update(plan)
        log("updated")
    except Exception:
        log("failed")
    finally:
        if client is not None:
            await client.close()
"#,
        "4562_ordinary_none.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f():
    client = None
    try:
        plan = await desired()
        client = await connect()
        handle = client.handle(plan)
        try:
            await handle.describe()
        except RPCError as error:
            if error.status != NOT_FOUND:
                raise
            try:
                await client.create(plan)
                log("created")
                return
            except AlreadyRunning:
                handle = client.handle(plan)
        await handle.update(plan)
        log("updated")
    except Exception:
        log("failed")
    finally:
        if client:
            await client.close()
"#,
        "4562_ordinary_truth_control.py",
    );
}

#[test]
fn ordinary_named_return_preserves_value_and_nested_try_controls() {
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error as error:
            log(error)
            return
    except OuterError:
        log("outer")
    finally:
        if client is not None:
            await client.close()
"#,
        "5368_named_bare_none.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error as error:
            log(error)
            return 7
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
"#,
        "5368_named_literal_truth.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error as error:
            try:
                try:
                    return result(error)
                except InnerError:
                    log("inner")
            except RecoveryError:
                log("recovery")
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
"#,
        "1573_named_value_two_tries.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error as error:
            try:
                yield render(error)
                return
            except RecoveryError:
                log("recovery")
    except OuterError:
        log("outer")
    finally:
        if client is not None:
            await client.close()
"#,
        "0217_named_generator_return.py",
    );
}

#[test]
fn ordinary_named_return_preserves_unnamed_and_no_return_controls() {
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error:
            log("error")
            return 7
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
"#,
        "unnamed_literal_control.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(client):
    try:
        try:
            await work()
        except Error as error:
            log(error)
            log("handled")
    except OuterError:
        log("outer")
    finally:
        if client is not None:
            await client.close()
"#,
        "named_no_return_control.py",
    );
}
