async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink([f"{key}:{value}" for key, value in rows].append(
            f"flag:{'yes' if ready else 'no'}"
        ))
