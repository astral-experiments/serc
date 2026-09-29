def target(
    p00, p01, p02, p03, p04, p05, p06,
    p07, p08, p09, p10, p11, p12, p13,
    p14, p15, p16, p17, p18, p19, p20,
    p21, p22, p23, p24, p25, p26, float,
    emit, convert,
):
    try:
        if p26 is None:
            raise ValueError("measurement unavailable")
        q0, q1, q2, q3, q4, q5 = p15.quantiles(
            "value",
            [0.0001, 0.001, 0.01, 0.10, 0.50, 0.90],
            float(p03),
        )
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
        emit(convert(q0))
        emit(convert(q1))
        emit(convert(q2))
        emit(convert(q3))
        emit(convert(q4))
        emit(convert(q5))
    finally:
        if p15 is not None:
            p15.close()
        if p21 is not None:
            p21.close()
        if p20 is not None:
            p20.close()
        if p18 is not None:
            p18.close()
        if p19 is not None:
            p19.close()
        if p17 is not None:
            p17.close()
        if p16 is not None:
            p16.close()
        p14.close()
        p13.close()
        p12.close()
        p11.close()
        p10.close()
        p09.close()
        p08.close()
        p07.close()
