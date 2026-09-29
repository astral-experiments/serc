def f(spark):
    records = None
    try:
        records = load(spark)
        results = records.collect()
    finally:
        if records is not None:
            records.unpersist()
        spark.stop()
    if len(results) != 1:
        raise ValueError("count")
    return results
