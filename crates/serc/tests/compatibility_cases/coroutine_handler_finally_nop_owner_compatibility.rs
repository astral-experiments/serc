use crate::common;

#[test]
fn coroutine_handler_finally_nop_owners_match_cpython_marshal() {
    let source = r#"async def method_cleanup(client):
    try:
        await client.start()
    except TimeoutError:
        return {"base": "timeout"}
    except OSError:
        return {"base": "os"}
    finally:
        await client.stop()
    return {}


async def attribute_cleanup(client):
    try:
        await client.start()
    except OSError as error:
        raise RuntimeError(error) from error
    finally:
        client.closed = True


async def global_cleanup(client):
    try:
        await client.start()
    except OSError:
        raise
    finally:
        cleanup(client)


async def bare_await_method(self):
    try:
        await source
    except Exception:
        recover()
        raise
    finally:
        self.cleanup()
"#;

    common::assert_matches_cpython_marshal(source, "coroutine_handler_finally_nop_owners.py");
}
