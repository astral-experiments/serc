async def finish(state, *, publish=True):
    session = await refresh(state)
    session_key = state.key
    if session is None or session_key is None:
        return False
    durable = False
    info: tuple[str, str] | None = None
    saved = not state.dirty
    try:
        if state.flush:
            await flush(state, force=True)
        elif state.chunk is not None and state.chunk.buffer:
            record("drop", extra={"key": session_key, "value": state.value, "size": len(state.chunk.buffer)})
        info = await upload(state)
        if info is not None:
            committed = await commit(state, info=info)
            durable = committed and available(state)
            if not durable:
                record_blocked(kind="state", reason="commit")
        else:
            durable = available(state)
    except PrimaryError:
        if info is not None:
            record_blocked(kind="state", reason="primary")
        raise
    except errors.Cancelled:
        if info is not None:
            record_blocked(kind="state", reason="cancelled")
        record_error("cancelled", extra={"key": session_key, "value": state.value})
        raise
    except Exception:
        if info is not None:
            record_blocked(kind="state", reason="failure")
        record_error("failure", extra={"key": session_key, "value": state.value})
    finally:
        if state.dirty:
            try:
                await session.save(condition=WriteMode.Unchanged)
                state.dirty = False
                saved = True
            except ConflictError:
                record("conflict", extra={"key": session_key})
                saved = False
            except Exception:
                record_error("save", extra={"key": session_key})
                saved = False
        if saved and durable and state.publish and publish:
            await notify(state)
        state.chunk = None
    return saved and durable
