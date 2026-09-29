def protected_loop_arm_return(command, items, as_list, emit):
    try:
        if command == "head":
            emit(None)
        elif command == "items":
            if not isinstance(items, list):
                raise ValueError("expected list")
            selected = [item for item in items if isinstance(item, dict)]
            if as_list:
                emit(selected)
            else:
                for item in selected:
                    emit(str(item.get("value", "")))
        elif command == "tail":
            emit(None)
        else:
            emit(False)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0


def protected_loop_arm_without_comprehension(command, items, as_list, emit):
    try:
        if command == "head":
            emit(None)
        elif command == "items":
            if as_list:
                emit(items)
            else:
                for item in items:
                    emit(item)
        elif command == "tail":
            emit(None)
        else:
            emit(False)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0
