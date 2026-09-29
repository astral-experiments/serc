for group in groups:
    with manager:
        for item in group:
            if item:
                work()
                pass
            followup()
