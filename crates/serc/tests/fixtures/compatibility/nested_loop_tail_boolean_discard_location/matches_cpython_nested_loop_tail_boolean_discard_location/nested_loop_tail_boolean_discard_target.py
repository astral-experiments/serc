def summarize_records():
    filename = "data/records.bin"
    records = load_records(filename)
    codec = get_codec("compact")
    lower = codec.encode("<field_0>", allowed_special="all")
    assert len(lower) == 1
    lower = lower[0]
    upper = codec.encode("<field_16383>", allowed_special="all")
    assert len(upper) == 1
    upper = upper[0]

    totals = []
    for fields in records:
        count = 0
        for field in fields:
            if not isinstance(field.data, int):
                continue
            if field.data >= lower and field.data <= upper:
                count += 1
        totals.append(count)
        if count >= 15_000:
            from helpers.runtime import get_worker_index

            get_worker_index() == 0 and __import__("observer").pause()

    import display.plot as plot

    plot.title("field count distribution")
    plot.hist(totals, bins=50)
    save_plot("data/field_counts.png")
