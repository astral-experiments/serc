def transform_batch(
    data, rate, setting_a=(1.0, 2.0), setting_b=(3.0, 4.0),
    setting_c=(5.0, 6.0), setting_d=(7.0, 8.0), setting_e=(9.0, 10.0),
    setting_f=(0.0, 1.0), minimum=100.0, maximum=1000.0,
    normalize=True, per_channel=False, per_item=True, generator=None,
    return_metadata=True,
):
    assert data.ndim in (2, 3), "expected two or three dimensions"
    device, dtype = data.device, data.dtype
    batch = data.size(0)

    def sample(value, *, is_integer=False):
        if isinstance(value, (tuple, list)) and len(value) == 2:
            low, high = float(value[0]), float(value[1])
            if is_integer:
                first, last = int(round(low)), int(round(high))
                return int(random_integer(first, last + 1, device=device, generator=generator))
            else:
                fraction = random_fraction(device=device, generator=generator)
                return low + (high - low) * float(fraction)
        return int(value) if is_integer else float(value)

    def process_one(item):
        a = sample(setting_a, is_integer=False)
        b = sample(setting_b, is_integer=False)
        c = sample(setting_c, is_integer=False)
        d = sample(setting_d, is_integer=False)
        e = sample(setting_e, is_integer=False)
        f = min(1.0, max(0.0, sample(setting_f, is_integer=False)))
        intermediate = transform(item, rate=rate, a=a, b=b, c=c, d=d,
                                 per_channel=per_channel, generator=generator)
        value = combine(item, intermediate, rate=rate, minimum=minimum,
                        maximum=maximum, e=e, normalize=normalize,
                        per_channel=per_channel, f=f, generator=generator)
        return value, {"a": a, "b": b, "c": c, "d": d, "e": e, "f": f}

    if not per_item:
        value, metadata = process_one(data)
        return (value, metadata) if return_metadata else value

    if data.ndim == 2:
        channels = 1
        inputs = data.unsqueeze(1)
    else:
        channels = data.size(1)
        inputs = data

    outputs, records = [], []
    for index in range(batch):
        item, record = process_one(inputs[index:index + 1, ...])
        outputs.append(item)
        records.append(record)
    value = concatenate(outputs, dim=0)

    return (
        (value if data.ndim == 3 else value[:, 0], records)
        if return_metadata
        else (value if data.ndim == 3 else value[:, 0])
    )
