def shared_pass_elif(kind, tool, emit):
    if kind == "delta":
        if tool == "search":
            emit("search")
        elif tool == "finalize":
            pass
    elif kind == "done":
        emit("done")
    emit("after")


def shared_pass_if(active, ready, emit):
    if active:
        if ready:
            pass
    else:
        emit("inactive")
    emit("after")


def adjacent_pass(active, emit):
    if active:
        pass
    else:
        emit("inactive")
    emit("after")


def sole_pass_fallthrough(active, finished, emit):
    if active:
        if finished:
            return "finished"
        else:
            pass
    else:
        emit("inactive")
    emit("after")


def protected_shared_pass(active, ready, emit):
    try:
        if active:
            if ready:
                pass
        else:
            emit("inactive")
        emit("after")
    except ValueError:
        emit("error")


def manager_pass(manager, emit):
    with manager:
        pass
    emit("after")


async def streaming_shared_pass(events, emit):
    async for kind, tool in events:
        if kind == "delta":
            if tool == "search":
                yield "search"
            elif tool == "finalize":
                pass
        elif kind == "done":
            yield "done"
        emit("after")
