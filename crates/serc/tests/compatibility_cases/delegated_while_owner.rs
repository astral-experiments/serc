use crate::common;

#[test]
fn async_delegated_inner_while_break() {
    common::assert_matches_cpython_marshal(
        r"async def f(convo, delegate):
    i = 0
    while i < len(convo.messages):
        message = convo.messages[i]
        if message.role == 0:
            i += 1
        elif message.role == 1:
            i += 1
            if delegate:
                first = True
                while i < len(convo.messages):
                    if convo.messages[i].role != 1:
                        i += 1
                    else:
                        break
            else:
                other()
",
        "async_delegated_inner_while_break.py",
    );
}

#[test]
fn async_delegated_inner_while_no_break_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(convo, delegate):
    i = 0
    while i < len(convo.messages):
        message = convo.messages[i]
        if message.role == 0:
            i += 1
        elif message.role == 1:
            i += 1
            if delegate:
                first = True
                while i < len(convo.messages):
                    if convo.messages[i].role != 1:
                        i += 1
                    else:
                        i += 1
            else:
                other()
",
        "async_delegated_inner_while_no_break_control.py",
    );
}

#[test]
fn async_delegated_inner_while_no_outer_loop_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(convo, delegate):
    i = 0
    message = convo.messages[i]
    if message.role == 0:
        i += 1
    elif message.role == 1:
        i += 1
        if delegate:
            first = True
            while i < len(convo.messages):
                if convo.messages[i].role != 1:
                    i += 1
                else:
                    break
        else:
            other()
",
        "async_delegated_inner_while_no_outer_loop_control.py",
    );
}

#[test]
fn async_delegated_inner_while_final_else_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(convo, delegate):
    i = 0
    while i < len(convo.messages):
        message = convo.messages[i]
        if message.role == 0:
            i += 1
        elif message.role == 1:
            i += 1
            if delegate:
                other()
            else:
                first = True
                while i < len(convo.messages):
                    if convo.messages[i].role != 1:
                        i += 1
                    else:
                        break
",
        "async_delegated_inner_while_final_else_control.py",
    );
}
