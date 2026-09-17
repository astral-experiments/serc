use crate::common;

#[test]
fn runtime_while_recovery_cleanup_keeps_normal_borrow_entry() {
    let source = r"def read_items(label, channel, state, done):
    try:
        while item := input_api.read(channel.fileno(), BLOCK_SIZE):
            state.accept(label, item)
    except (OSError, ValueError):
        state.recover()
    finally:
        try:
            channel.close()
        except OSError:
            state.recover()
        done.set()
        state.changed.set()


def write_items(channel, data, state, done):
    try:
        view = memoryview(data)
        offset = 0
        while offset < len(view):
            try:
                written = input_api.write(channel.fileno(), view[offset:])
            except InterruptedError:
                continue
            if written == 0:
                state.recover()
                return
            offset += written
    except BrokenPipeError:
        pass
    except OSError as error:
        if error.errno != codes.CLOSED:
            state.recover()
    finally:
        try:
            channel.close()
        except (BrokenPipeError, OSError):
            pass
        done.set()
        state.changed.set()
";
    common::assert_matches_cpython_marshal(source, "runtime_while_recovery_cleanup.py");
}
