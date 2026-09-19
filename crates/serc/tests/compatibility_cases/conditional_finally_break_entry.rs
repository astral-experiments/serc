use crate::common;

#[test]
fn matches_cpython_marshal_for_conditional_finally_break_entry() {
    let source = r#"async def transfer(source, sink, *, close=True, chunk_size=64 * 1024,
                   debug=False, debug_raw=False, label=""):
    framer = make_framer() if debug else None
    try:
        while True:
            data = await source.read(chunk_size)
            if not data:
                break
            if debug_raw:
                report(label, data)
            if framer is not None:
                framer.feed(data)
            sink.write(data)
            await sink.drain()
    except Cancelled:
        raise
    except (Closed, Disconnected):
        pass
    except Exception as error:
        report(error)
    finally:
        if close:
            try:
                sink.close()
            except Exception:
                pass
"#;
    common::assert_matches_cpython_marshal(source, "conditional_finally_break_target.py");
}

#[test]
fn matches_cpython_marshal_for_conditional_finally_break_call_control() {
    let source = r#"async def transfer(source, sink, *, close=True, chunk_size=64 * 1024,
                   debug=False, debug_raw=False, label=""):
    framer = make_framer() if debug else None
    try:
        while True:
            data = await source.read(chunk_size)
            if not data:
                break
            if debug_raw:
                report(label, data)
            if framer is not None:
                framer.feed(data)
            sink.write(data)
            await sink.drain()
    except Cancelled:
        raise
    except (Closed, Disconnected):
        pass
    except Exception as error:
        report(error)
    finally:
        observe_cleanup(close)
        if close:
            try:
                sink.close()
            except Exception:
                pass
"#;
    common::assert_matches_cpython_marshal(source, "conditional_finally_break_call_control.py");
}
