def read_records(reader, seen_lock, seen, stats, output):
    with reader() as records:
        for line_number, record in enumerate(records):
            raw_key = record.get('key')
            if raw_key is not None:
                if isinstance(raw_key, (list, tuple)):
                    key = tuple(raw_key)
                else:
                    key = (str(raw_key),)
                with seen_lock:
                    if key in seen:
                        stats.increment('duplicate')
                        continue
                    else:
                        seen.add(key)
            record['line'] = line_number
            output.put(record)
            stats.increment('read')
