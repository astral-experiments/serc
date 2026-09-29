def f(items, active, output):
    for item in items:
        if item.startswith("start"):
            output.append(item)
        elif item == "skip":
            continue
        elif ("a" in item and "b" in item) or item.startswith("c") or item.strip().startswith("d") or "e" in item:
            assert active
        elif item == "work":
            output.append(item)
        elif "x" in item or "y" in item or "z" in item:
            assert active
        else:
            report(item)
    return output
