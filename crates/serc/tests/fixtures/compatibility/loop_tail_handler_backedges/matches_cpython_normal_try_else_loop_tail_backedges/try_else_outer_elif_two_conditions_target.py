def collect(widgets, decode):
    first = set()
    second = set()
    for widget in widgets:
        if widget.bindings is not None:
            for binding in widget.bindings.values():
                first.add(binding.identifier)
                if binding.version is not None:
                    second.add(binding.version)
        elif widget.kind == "form":
            try:
                value = decode(widget.output)
            except Exception:
                pass
            else:
                if value.identifier is not None:
                    first.add(value.identifier)
                if value.version is not None:
                    second.add(value.version)
    return first, second
