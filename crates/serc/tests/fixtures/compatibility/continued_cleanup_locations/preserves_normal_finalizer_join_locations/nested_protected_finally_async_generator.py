async def stream_pages(pages, token, publish):
    while True:
        try:
            stream = pages()
            try:
                async for item in stream:
                    yield item
            finally:
                try:
                    await stream.aclose()
                except Exception:
                    warning()
                if token is None and stream.version is not None:
                    await publish(stream.version)
        except LookupError as error:
            if error.args:
                raise
            continue
        token = stream.next_token
        if not token:
            return
