def nested_for_return(outer, inner, guard, values, sink, result):
    if outer:
        if inner:
            sink(result)
        else:
            if guard:
                for value in values:
                    if value not in result:
                        sink(value)
    elif guard:
        sink(result)
    return result


def event_shape(event, issues, sink):
    kind = event.get("kind")
    if kind == "request":
        if event.get("accepted"):
            if event.get("served"):
                sink(issues)
            else:
                if not event.get("first"):
                    sink(issues)
                if event.get("first") and event.get("first") == event.get("last"):
                    for field in ("first", "last"):
                        if field not in issues:
                            issues.append(field)
        elif event.get("requested"):
            sink(issues)
        elif event.get("observed"):
            if "context" not in issues:
                issues.append("context")
    elif kind == "attempt":
        sink(issues)
    return issues


def for_completion_control(values, sink, result):
    for value in values:
        if value not in result:
            sink(value)
    return result
