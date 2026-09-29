def f(batches, make, emit):
    for batch in batches:
        for item in batch:
            try:
                source = make(item)
                if source is None:
                    continue
                iterator = source.instructions()
                while iterator.more():
                    instruction = iterator.next()
                    for reference in instruction.refs():
                        try:
                            kind = reference.kind()
                            if kind is not None and kind.is_call():
                                target = reference.target()
                                if target is not None:
                                    emit(item, target)
                        except Exception: pass
            except Exception: continue
