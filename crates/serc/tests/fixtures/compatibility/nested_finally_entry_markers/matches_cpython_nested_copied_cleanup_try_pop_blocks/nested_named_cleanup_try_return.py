def probe(inner, outer, report):
    try:
        try:
            return 0
        finally:
            try:
                result = inner()
            except Exception as error:
                report(error)
    finally:
        try:
            outer()
        except Exception as error:
            report(error)
