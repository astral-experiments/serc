use crate::common;

#[test]
fn matches_cpython_marshal_for_generator_with_finally_condition_owners() {
    let source = r"def single_cleanup(manager, resource):
    with manager:
        try:
            yield resource
        finally:
            if resource is not None:
                resource.close()

def double_cleanup(manager, first, second):
    with manager:
        try:
            yield first
        finally:
            if first is not None:
                first.close()
            if second is not None:
                second.join(timeout=2)

def attribute_cleanup(manager, state, stack):
    with manager:
        try:
            yield state
        finally:
            if state.debug:
                stack.pop_all()
                notify(state)

async def async_generator_cleanup(manager, resource):
    with manager:
        try:
            yield resource
        finally:
            if resource is not None:
                resource.close()

def leading_cleanup(manager, server, thread):
    with manager:
        try:
            yield server
        finally:
            server.shutdown()
            server.server_close()
            if thread is not None:
                thread.join(timeout=2)
";
    common::assert_matches_cpython_marshal(source, "generator_with_finally_condition_owners.py");
}
