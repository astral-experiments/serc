def compare_levels(rows):
    changed = False
    for current, middle, target in rows:
        if target < current:
            changed = True
        elif target > current:
            if middle is not None:
                if middle < current:
                    changed = True
                elif current < middle <= target:
                    changed = True
        else:
            if middle is not None and middle < current:
                changed = True
    return changed
