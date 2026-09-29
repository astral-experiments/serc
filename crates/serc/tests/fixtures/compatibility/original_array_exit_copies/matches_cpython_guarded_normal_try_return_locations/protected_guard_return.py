def protected_guard_return(command, get_result, emit):
    try:
        result = get_result()
        if command == "head":
            emit(result)
        elif command == "check":
            emit(result)
            if str(result["status"].get("state", "")).upper() != "READY":
                return 2
        elif command == "tail":
            emit(result)
        else:
            emit(None)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0
