async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink(rows.append(
            f"flag:{'yes' if ready else 'no'}"
        ))
