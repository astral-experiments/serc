def search(api):
    found = False
    try:
        while api.pending():
            try:
                result = api.run(("first", "second"), enabled=True)
                values = result.values()
                if not result.ready():
                    pass
                else:
                    for value in values:
                        if api.matches(value):
                            api.accept(value)
                            found = True
                            break
                    if found:
                        break
            except LookupError:
                pass
            api.wait()
    except Exception:
        return False
    return found
