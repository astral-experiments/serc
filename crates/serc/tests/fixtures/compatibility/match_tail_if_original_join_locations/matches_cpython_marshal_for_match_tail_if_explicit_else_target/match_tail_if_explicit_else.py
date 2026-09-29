def transform(parts, role, supports_audio, replace, author):
    changed = False
    for part in parts or []:
        match part:
            case Audio():
                changed = True
                if role == "user" and supports_audio:
                    if part.expired:
                        if not replace:
                            report(
                                "expired",
                                part.key,
                                author.role if author is not None else None,
                                part.pointer,
                            )
                    else:
                        accept(part)
            case Image():
                visit(part)
    return changed
