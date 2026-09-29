def select_values(rows, fraction=0.10):
    values = (
        rows.map(lambda item: item.get("value"))
        .filter(lambda value: value is not None)
        .map(lambda value: (float(value),))
        .persist()
    )
    threshold = None
    try:
        try:
            values.first()
        except Exception:
            threshold = None
        else:
            session = Session.builder.config(conf=rows.context.getConf()).getOrCreate()
            table = session.createDataFrame(values, schema=["value"])
            samples = table.approxQuantile("value", [fraction], 0.001)
            threshold = samples[0] if samples else None
    finally:
        values.unpersist()
    if threshold is None:
        return rows.context.parallelize([])
    return rows.filter(lambda item: item.get("value") is not None and float(item["value"]) <= threshold)
