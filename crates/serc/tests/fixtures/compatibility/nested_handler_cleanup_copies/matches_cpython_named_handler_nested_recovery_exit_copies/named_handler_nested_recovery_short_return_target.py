def choose(value, first, fallback):
    result = {}
    try:
        result["value"] = primary(value)
    except ValueError as error:
        if first:
            result["value"] = "exists"
        elif fallback:
            try:
                result["value"] = secondary(value)
            except ValueError as recovery:
                if can_recover(recovery):
                    result["value"] = "recovered"
                else:
                    raise
        else:
            raise
    return result
