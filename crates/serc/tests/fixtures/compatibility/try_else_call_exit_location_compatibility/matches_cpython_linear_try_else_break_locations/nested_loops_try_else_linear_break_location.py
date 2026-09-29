def run(groups, roots, copy, exists):
    restored = []
    for group in groups:
        for root in roots:
            try:
                copy(root, group)
            except Exception as error:
                message = str(error)
            else:
                if exists(group):
                    ready = True
                    restored.append(group)
                    report(root, group)
                    break
            retry(root, group)
    return restored
