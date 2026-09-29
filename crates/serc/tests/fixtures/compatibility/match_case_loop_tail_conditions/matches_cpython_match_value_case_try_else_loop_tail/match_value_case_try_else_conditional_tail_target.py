def convert(rows, page, decode):
    for fields in rows:
        for field, payload in fields:
            match field:
                case "name":
                    page.name = payload
                case "counters":
                    try:
                        counters = decode(payload)
                    except Exception:
                        pass
                    else:
                        if value := counters.get("created"):
                            page.created = value
                            page.modified = value
                        if value := counters.get("modified"):
                            page.modified = value
                case "date":
                    page.date = payload
                case _:
                    page.other = payload
    return page
