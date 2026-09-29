def make_handler(cleanup):
    async def handle(request):
        async def generate():
            phase = request.phase
            token = ""
            completed_token = ""
            text = ""
            state = "pending"
            reason = ""
            initial_token = ""
            initial_text = ""
            options = list(request.options)
            stream = None
            error = None
            try:
                yield encode({"kind": "context", "phase": phase})
                client = make_client()
                try:
                    stream = await client.open(request)
                except Exception as exc:
                    if "retry" not in str(exc):
                        raise
                    stream = await client.open(request, retry=True)
                async for event in stream:
                    if event.kind == "created":
                        token = event.token
                        yield encode({"kind": "created", "token": token})
                    elif event.kind == "completed":
                        completed_token = token
                        text = event.text
                        yield encode({"kind": "completed", "text": text})
                        if request.stop_early:
                            return
                    elif event.kind == "failed":
                        yield encode({"kind": "error", "detail": event.detail})
            except Exception as exc:
                error = str(exc)
                yield encode({"kind": "error", "detail": error})
            finally:
                cleanup(
                    payload={
                        "phase": phase,
                        "token": completed_token or token,
                        "text": text.strip(),
                        "state": state,
                        "reason": reason,
                        "initial_token": initial_token,
                        "initial_text": initial_text,
                        "options": [option.serialize() for option in options],
                        "stale": request.stale,
                        "sequence": request.sequence,
                        "version": request.version,
                        "stop": request.stop,
                        "asked": request.asked,
                        "count": request.count,
                        "limit": request.limit,
                        "followup_count": request.followup_count,
                        "followup_limit": request.followup_limit,
                    },
                    prompt=(welcome(request) if request.welcome else fallback(request)),
                    error=error,
                )
                if stream is not None:
                    try:
                        await stream.aclose()
                    except Exception:
                        pass
        return generate()
    return handle
