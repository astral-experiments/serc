def check_changes(changes, action):
    count = 0
    valid = True
    for change, old, new in changes:
        match action:
            case "archive":
                count += 1
            case "mark as read":
                count += 1
            case "move to trash":
                if change == "added" and old is None and new == {"TRASH"}:
                    count += 1
                elif not (change == "removed" and old == {"INBOX"} and new is None):
                    valid = False
                    break
    return count, valid
