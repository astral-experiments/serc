def process(manager, batches, Plan, Task, load, emit, unexpected):
    with manager:
        while batches:
            for item in batches.pop():
                try:
                    result = load(item)
                except Exception as error:
                    match item:
                        case Plan() as planned:
                            prepared = None
                            emit(planned, error)
                            status = "failed"
                        case Task() as task:
                            emit(task, error)
                            continue
                        case impossible:
                            unexpected(impossible)
                else:
                    match result:
                        case Plan() as prepared:
                            status = "ready"
                        case bool() as changed:
                            emit(changed)
                            continue
                        case impossible_result:
                            unexpected(impossible_result)
                emit(prepared, status)
                emit("completed", item)
