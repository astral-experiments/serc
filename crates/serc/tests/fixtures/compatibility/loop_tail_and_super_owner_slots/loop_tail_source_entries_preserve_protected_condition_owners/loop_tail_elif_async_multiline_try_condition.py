async def f(groups, sink, receiver, first, second):
    for group in groups:
        for item in group.items:
            if item.kind == KIND:
                sink.append(receiver.work(
                    first,
                    second,
                    key=item.key,
                    value=item.extra.get("value"),
                ))
                try:
                    if item.extra and "value" in item.extra:
                        await consume(item)
                except Exception:
                    recover()
            elif item.kind == OTHER:
                sink.append(receiver.other(first))
