def f(mode, groups):
    preview = []
    used = [0 for group in groups]
    if mode:
        last = len(groups) - 1
        while len(preview) < 3 and last >= 0:
            group = groups[last]
            index = used[last]
            if index < len(group):
                preview.append(group[index])
                used[last] += 1
            else:
                break
    else:
        while len(preview) < 3:
            added = False
            for index, group in enumerate(groups):
                if group:
                    preview.append(group[0])
                    added = True
                    if len(preview) >= 3:
                        break
            if not added:
                break
    return preview, used, mode
