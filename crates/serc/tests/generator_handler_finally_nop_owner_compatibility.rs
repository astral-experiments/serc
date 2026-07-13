#[allow(dead_code)]
mod common;

#[test]
fn generator_handler_finally_nop_owners_match_cpython_marshal() {
    let source = r"def bare_cleanup():
    try:
        yield source
    except:
        raise
    finally:
        cleanup()


def named_cleanup(trace):
    try:
        yield trace
    except Exception as error:
        trace.set_error(error)
        raise
    finally:
        trace.reset()


def assigned_cleanup(limit):
    try:
        executor = make_executor(limit)
        yield executor
    except KeyboardInterrupt:
        clear_workers()
        raise
    finally:
        executor.shutdown(wait=False)


def attributed_cleanup(client):
    try:
        yield client
    except OSError:
        client.abort()
        raise
    finally:
        client.close()
";

    common::assert_matches_cpython_marshal(source, "generator_handler_finally_nop_owners.py");
}

#[test]
fn generator_handler_finally_nop_owner_edge_shapes_match_cpython_marshal() {
    let source = r"def explicit_raise_cleanup():
    try:
        yield source
    except Exception as error:
        raise error
    finally:
        cleanup()


def raise_from_cleanup():
    try:
        yield source
    except Exception as error:
        raise error from error.__cause__
    finally:
        cleanup()


def assigned_yield_cleanup():
    try:
        received = yield source
        yield received
    except Exception:
        raise
    finally:
        cleanup()


def handler_yield_cleanup():
    try:
        yield source
    except Exception:
        yield fallback
        raise
    finally:
        cleanup()


def finally_yield_cleanup():
    try:
        yield source
    except Exception:
        raise
    finally:
        yield cleanup
";

    common::assert_matches_cpython_marshal(source, "generator_handler_finally_nop_owner_edges.py");
}
