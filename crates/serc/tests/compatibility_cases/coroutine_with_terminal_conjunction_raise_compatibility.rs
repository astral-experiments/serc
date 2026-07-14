use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_with_terminal_conjunction_raise() {
    let source = r"async def terminal_attributes(manager, state, prepare):
    with manager:
        await prepare()
        if state.data and state.receiver_closed:
            raise Error

async def terminal_names(manager, first, second, prepare):
    with manager:
        await prepare()
        if first and second:
            raise Error

async def lockstep_shape(manager, state, data, wait_for):
    with manager:
        if state.sender_closed:
            raise Closed
        if state.receiver_closed:
            raise Broken
        assert not state.data
        state.data += data
        state.notify()
        await wait_for(lambda: state.data == b'')
        if state.sender_closed:
            raise Closed
        if state.data and state.receiver_closed:
            raise Broken

async def disjunction_control(manager, state, prepare):
    with manager:
        await prepare()
        if state.data or state.receiver_closed:
            raise Error
";
    common::assert_matches_cpython_marshal(source, "coroutine_with_terminal_conjunction_raise.py");
}
